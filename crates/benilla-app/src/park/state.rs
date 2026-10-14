use benilla_protocol::park::{Definition, Message, Object, Part, Pose, MAX_OBJECTS_PER_MAP};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) type Key = (u64, u32);
#[derive(Resource, Default)]
pub(crate) struct ParkWorld {
    pub(super) objects: HashMap<Key, Object>,
    pub(super) map: u32,
    pub(super) instance: u32,
    pub(crate) generation: u64,
    pub(super) ready: bool,
    pub(super) capable: bool,
    pub(super) last_sync: f32,
    staging: Option<Staging>,
    removed: HashSet<Key>,
    removed_order: VecDeque<Key>,
}
enum StageKind {
    Snapshot(u32),
    Change(u64, u32),
}
struct Incoming {
    object: Object,
    parts: Vec<Option<Part>>,
}
struct Staging {
    kind: StageKind,
    map: u32,
    instance: u32,
    expected_objects: usize,
    expected_removed: usize,
    seen: HashSet<Key>,
    removed: HashSet<Key>,
    objects: HashMap<Key, Object>,
    incoming: HashMap<Key, Incoming>,
}
impl ParkWorld {
    pub(crate) fn replay_objects(&self) -> Vec<Object> {
        let mut objects: Vec<_> = self.objects.values().cloned().collect();
        objects.sort_by_key(|o| (o.owner, o.id));
        objects
    }
    /// The authoritative map and instance used by shared park objects and private markers.
    pub(crate) fn scope(&self) -> (u32, u32) {
        (self.map, self.instance)
    }
    pub(super) fn clear(&mut self) {
        self.objects.clear();
        self.staging = None;
        self.removed.clear();
        self.removed_order.clear();
        self.ready = false;
        self.generation = self.generation.wrapping_add(1);
    }
    fn tombstone(&mut self, key: Key) {
        if self.removed.insert(key) {
            self.removed_order.push_back(key);
        }
        while self.removed_order.len() > 4096 {
            if let Some(old) = self.removed_order.pop_front() {
                self.removed.remove(&old);
            }
        }
    }
    fn finish(&mut self, kind: StageKind) -> Result<(), &'static str> {
        let Some(stage) = self.staging.take() else {
            return Err("Unexpected park transaction end");
        };
        let matches = match (stage.kind, kind) {
            (StageKind::Snapshot(a), StageKind::Snapshot(b)) => a == b,
            (StageKind::Change(a, b), StageKind::Change(c, d)) => a == c && b == d,
            _ => false,
        };
        if !matches
            || stage.seen.len() != stage.expected_objects
            || stage.removed.len() != stage.expected_removed
            || !stage.incoming.is_empty()
            || stage.objects.len() > MAX_OBJECTS_PER_MAP
        {
            self.ready = false;
            return Err("Incomplete park transaction");
        }
        self.map = stage.map;
        self.instance = stage.instance;
        self.objects = stage.objects;
        for key in stage.removed {
            self.tombstone(key);
        }
        self.ready = true;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    pub(super) fn receive(&mut self, message: &Message) -> Result<(), &'static str> {
        match message {
            Message::Begin {
                snapshot,
                map,
                instance,
                count,
            } => {
                self.staging = Some(Staging {
                    kind: StageKind::Snapshot(*snapshot),
                    map: *map,
                    instance: *instance,
                    expected_objects: usize::from(*count),
                    expected_removed: 0,
                    seen: HashSet::new(),
                    removed: HashSet::new(),
                    objects: HashMap::new(),
                    incoming: HashMap::new(),
                });
                self.removed.clear();
                self.removed_order.clear();
                self.ready = false;
            }
            Message::ChangeBegin {
                request,
                owner,
                edits,
                drops,
            } => {
                if self.staging.is_some() {
                    return Err("Overlapping park transactions");
                }
                self.staging = Some(Staging {
                    kind: StageKind::Change(*owner, *request),
                    map: self.map,
                    instance: self.instance,
                    expected_objects: usize::from(*edits),
                    expected_removed: usize::from(*drops),
                    seen: HashSet::new(),
                    removed: HashSet::new(),
                    objects: self.objects.clone(),
                    incoming: HashMap::new(),
                });
            }
            Message::Object {
                object, part_count, ..
            } => {
                let Some(stage) = &mut self.staging else {
                    return Err("Object outside park transaction");
                };
                let key = (object.owner, object.id);
                if object.map != stage.map
                    || object.instance != stage.instance
                    || !stage.seen.insert(key)
                    || stage.seen.len() > stage.expected_objects
                {
                    return Err("Invalid park object scope or count");
                }
                if let StageKind::Change(owner, _) = stage.kind {
                    if object.owner != owner {
                        return Err("Invalid park transaction owner");
                    }
                }
                if *part_count == 0 {
                    stage.objects.insert(key, object.clone());
                } else {
                    stage.incoming.insert(
                        key,
                        Incoming {
                            object: object.clone(),
                            parts: vec![None; usize::from(*part_count)],
                        },
                    );
                }
            }
            Message::ObjectPart {
                owner,
                id,
                revision,
                index,
                part,
            } => {
                let Some(stage) = &mut self.staging else {
                    return Err("Part outside park transaction");
                };
                let key = (*owner, *id);
                let Some(incoming) = stage.incoming.get_mut(&key) else {
                    return Err("Unknown fused object part");
                };
                if incoming.object.revision != *revision {
                    return Err("Mismatched fused object revision");
                }
                let Some(slot) = incoming.parts.get_mut(usize::from(*index)) else {
                    return Err("Invalid fused object part index");
                };
                if slot.is_some() {
                    return Err("Duplicate fused object part");
                }
                *slot = Some(part.clone());
                if incoming.parts.iter().all(Option::is_some) {
                    let mut incoming = stage.incoming.remove(&key).unwrap();
                    incoming.object.parts =
                        incoming.parts.into_iter().map(Option::unwrap).collect();
                    incoming
                        .object
                        .definition()
                        .validate()
                        .map_err(|_| "Invalid fused object definition")?;
                    stage.objects.insert(key, incoming.object);
                }
            }
            Message::End { snapshot } => self.finish(StageKind::Snapshot(*snapshot))?,
            Message::ChangeEnd { owner, request } => {
                self.finish(StageKind::Change(*owner, *request))?
            }
            Message::Removed { request, owner, id } => {
                let key = (*owner, *id);
                if let Some(stage) = &mut self.staging {
                    if !matches!(stage.kind,StageKind::Change(source,nonce) if source==*owner&&nonce==*request)
                        || !stage.removed.insert(key)
                        || stage.removed.len() > stage.expected_removed
                    {
                        return Err("Invalid park transaction deletion");
                    }
                    stage.objects.remove(&key);
                } else {
                    self.objects.remove(&key);
                    self.tombstone(key);
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            Message::ClearOwner { owner } => {
                let keys: Vec<Key> = self
                    .objects
                    .keys()
                    .filter(|key| key.0 == *owner)
                    .copied()
                    .collect();
                for key in keys {
                    self.objects.remove(&key);
                    self.tombstone(key);
                }
                self.generation = self.generation.wrapping_add(1);
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Change {
    pub id: Option<u32>,
    pub before: Option<Definition>,
    pub after: Option<Definition>,
}
pub(super) type ChangeSet = Vec<Change>;
#[derive(Clone)]
pub(super) enum HistoryAction {
    Push(ChangeSet),
    Undo(usize),
    Redo(usize),
}
pub(super) struct Pending {
    pub request: u32,
    pub action: HistoryAction,
    pub sent_at: f32,
    pub bindings: Vec<usize>,
    pub assigned: HashMap<u16, u32>,
}
#[derive(Resource, Default)]
pub(crate) struct ParkEditor {
    pub(crate) active: bool,
    pub(super) toggle_requested: bool,
    pub(super) selected: Option<Key>,
    pub(super) selection: Vec<Key>,
    pub(super) tool: super::math::Tool,
    pub(super) axis: Option<usize>,
    pub(super) local: bool,
    pub(super) preview: Option<(benilla_protocol::park::Model, Pose)>,
    pub(super) drafts: Vec<Definition>,
    pub(super) preview_parts: Vec<Part>,
    pub(super) previews: HashMap<Key, Definition>,
    pub(super) preview_sources: Vec<(Key, Definition)>,
    pub(super) preview_root: Option<(Pose, Pose)>,
    pub(super) drag: Option<super::input::Drag>,
    pub(super) saved_camera: Option<Transform>,
    pub(super) camera: Transform,
    pub(super) status: String,
    pub(super) history: Vec<ChangeSet>,
    pub(super) cursor: usize,
    pub(super) pending: Option<Pending>,
    pub(super) next_request: u32,
}
impl ParkEditor {
    pub(super) fn clear_preview(&mut self) {
        self.drafts.clear();
        self.preview = None;
        self.preview_parts.clear();
        self.previews.clear();
        self.preview_sources.clear();
        self.preview_root = None;
        self.drag = None;
    }
    pub(super) fn select(&mut self, key: Option<Key>, toggle: bool) {
        if toggle {
            if let Some(key) = key {
                if self.selection.contains(&key) {
                    self.selection.retain(|k| *k != key);
                } else {
                    self.selection.push(key);
                }
            }
        } else {
            self.selection = key.into_iter().collect();
        }
        self.selected = self.selection.last().copied();
    }
    pub(super) fn prune(&mut self, world: &ParkWorld) {
        if self.pending.is_none() {
            self.selection.retain(|key| world.objects.contains_key(key));
            self.selected = self.selection.last().copied();
        }
    }
    pub(super) fn target_pose(&self, world: &ParkWorld) -> Option<Pose> {
        if let Some((_, result)) = self.preview_root {
            return Some(result);
        }
        if !self.drafts.is_empty() {
            let centre = self
                .drafts
                .iter()
                .fold(Vec3::ZERO, |sum, d| sum + Vec3::from_array(d.pose.position))
                / self.drafts.len() as f32;
            return Some(Pose {
                position: centre.to_array(),
                rotation: self.drafts[0].pose.rotation,
                ..default()
            });
        }
        if let Some((_, pose)) = self.preview {
            return Some(pose);
        }
        let primary = world.objects.get(&self.selected?)?;
        let mut centre = Vec3::ZERO;
        let mut count = 0;
        for key in &self.selection {
            if let Some(object) = world.objects.get(key) {
                centre += Vec3::from_array(object.pose.position);
                count += 1;
            }
        }
        if count == 0 {
            return Some(primary.pose);
        }
        Some(Pose {
            position: (centre / count as f32).to_array(),
            rotation: primary.pose.rotation,
            ..default()
        })
    }
    pub(super) fn acknowledged(&mut self, message: &Message, owner: u64) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        match message {
            Message::Object {
                request,
                slot,
                object,
                ..
            } if *request == pending.request && object.owner == owner => {
                pending.assigned.insert(*slot, object.id);
                return;
            }
            Message::ChangeEnd {
                request,
                owner: source,
            } if *request == pending.request && *source == owner => {}
            _ => return,
        }
        let pending = self.pending.take().unwrap();
        let mut changes = match &pending.action {
            HistoryAction::Push(changes) => changes.clone(),
            HistoryAction::Undo(index) | HistoryAction::Redo(index) => self.history[*index].clone(),
        };
        for (slot, &index) in pending.bindings.iter().enumerate() {
            if let Some(&new) = pending.assigned.get(&(slot as u16)) {
                if let Some(old) = changes[index].id {
                    if old != new {
                        for change in self.history.iter_mut().flatten() {
                            if change.id == Some(old) {
                                change.id = Some(new);
                            }
                        }
                    }
                }
                changes[index].id = Some(new);
            }
        }
        let undo = matches!(pending.action, HistoryAction::Undo(_));
        let only_updates = changes
            .iter()
            .all(|change| change.before.is_some() && change.after.is_some());
        if !only_updates || self.selection.is_empty() {
            self.selection = changes
                .iter()
                .filter(|change| {
                    if undo {
                        change.before.is_some()
                    } else {
                        change.after.is_some()
                    }
                })
                .filter_map(|change| change.id.map(|id| (owner, id)))
                .collect();
            self.selected = self.selection.last().copied();
        }
        match pending.action {
            HistoryAction::Push(_) => {
                self.history.truncate(self.cursor);
                self.history.push(changes);
                if self.history.len() > 128 {
                    self.history.remove(0);
                }
                self.cursor = self.history.len();
            }
            HistoryAction::Undo(index) => {
                self.history[index] = changes;
                self.cursor = index;
            }
            HistoryAction::Redo(index) => {
                self.history[index] = changes;
                self.cursor = index + 1;
            }
        }
        self.clear_preview();
        self.status = "Saved for this login session".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use benilla_protocol::park::Model;
    fn object(owner: u64, id: u32) -> Object {
        Object {
            owner,
            id,
            revision: 1,
            map: 1,
            instance: 0,
            model: Model::Box,
            asset: 0,
            pose: Pose::default(),
            parts: Vec::new(),
        }
    }
    fn header(object: Object) -> Message {
        Message::Object {
            request: 8,
            slot: 0,
            part_count: 0,
            object,
        }
    }
    #[test]
    fn snapshot_is_atomic_and_requires_every_object() {
        let mut world = ParkWorld::default();
        world
            .receive(&Message::Begin {
                snapshot: 4,
                map: 1,
                instance: 0,
                count: 2,
            })
            .unwrap();
        world.receive(&header(object(1, 1))).unwrap();
        assert!(world.objects.is_empty());
        assert!(world.receive(&Message::End { snapshot: 4 }).is_err());
        assert!(!world.ready);
        world
            .receive(&Message::Begin {
                snapshot: 5,
                map: 1,
                instance: 0,
                count: 1,
            })
            .unwrap();
        world.receive(&header(object(1, 1))).unwrap();
        world.receive(&Message::End { snapshot: 5 }).unwrap();
        assert!(world.ready);
        assert_eq!(world.objects.len(), 1);
    }
    #[test]
    fn grouped_changes_do_not_expose_partial_moves() {
        let mut world = ParkWorld {
            map: 1,
            ready: true,
            ..default()
        };
        world.objects.insert((1, 1), object(1, 1));
        world.objects.insert((1, 2), object(1, 2));
        world
            .receive(&Message::ChangeBegin {
                owner: 1,
                request: 8,
                edits: 2,
                drops: 0,
            })
            .unwrap();
        let mut moved = object(1, 1);
        moved.pose.position[1] = 10.;
        world.receive(&header(moved)).unwrap();
        assert_eq!(world.objects[&(1, 1)].pose.position[1], 0.);
        world.receive(&header(object(1, 2))).unwrap();
        world
            .receive(&Message::ChangeEnd {
                owner: 1,
                request: 8,
            })
            .unwrap();
        assert_eq!(world.objects[&(1, 1)].pose.position[1], 10.);
    }
    #[test]
    fn ctrl_selection_toggles_and_keeps_last_selected_as_reference() {
        let mut editor = ParkEditor::default();
        editor.select(Some((1, 1)), false);
        editor.select(Some((1, 2)), true);
        assert_eq!(editor.selection.len(), 2);
        assert_eq!(editor.selected, Some((1, 2)));
        editor.select(Some((1, 2)), true);
        assert_eq!(editor.selected, Some((1, 1)));
    }
    #[test]
    fn other_players_acknowledgments_do_not_complete_our_edit() {
        let definition = Definition::primitive(Model::Box, Pose::default());
        let mut editor = ParkEditor {
            pending: Some(Pending {
                request: 8,
                action: HistoryAction::Push(vec![Change {
                    id: None,
                    before: None,
                    after: Some(definition),
                }]),
                sent_at: 0.,
                bindings: vec![0],
                assigned: HashMap::new(),
            }),
            ..default()
        };
        editor.acknowledged(&header(object(2, 2)), 1);
        editor.acknowledged(
            &Message::ChangeEnd {
                owner: 2,
                request: 8,
            },
            1,
        );
        assert!(editor.pending.is_some());
        editor.acknowledged(&header(object(1, 4)), 1);
        editor.acknowledged(
            &Message::ChangeEnd {
                owner: 1,
                request: 8,
            },
            1,
        );
        assert!(editor.pending.is_none());
        assert_eq!(editor.selected, Some((1, 4)));
        assert_eq!(editor.cursor, 1);
    }
    #[test]
    fn alignment_preserves_the_unchanged_reference_in_multi_selection() {
        let before = object(1, 1).definition();
        let mut after = before.clone();
        after.pose.position[1] = 2.;
        let mut editor = ParkEditor {
            selection: vec![(1, 1), (1, 2)],
            selected: Some((1, 2)),
            pending: Some(Pending {
                request: 8,
                action: HistoryAction::Push(vec![Change {
                    id: Some(1),
                    before: Some(before),
                    after: Some(after),
                }]),
                sent_at: 0.,
                bindings: vec![0],
                assigned: HashMap::new(),
            }),
            ..default()
        };
        editor.acknowledged(&header(object(1, 1)), 1);
        editor.acknowledged(
            &Message::ChangeEnd {
                owner: 1,
                request: 8,
            },
            1,
        );
        assert_eq!(editor.selection, vec![(1, 1), (1, 2)]);
        assert_eq!(editor.selected, Some((1, 2)));
    }
}
