//! Controller polling stays independent of render and collision work.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use skate_host::bridge::{ControllerTransport, InputFrame};

use super::flag::gesture::{Action, Gesture};
use super::input_buffer::{InputBuffer, Sample, MAX_AGE};

const POLL_PERIOD: Duration = Duration::from_millis(4);
const EDGE_CAPACITY: usize = 32;
const FLAG_ACTION_AGE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(super) struct Edge {
    pub(super) at: Instant,
    pub(super) pressed: u16,
    pub(super) held: u16,
}

#[derive(Default)]
struct State {
    #[cfg(feature = "dev")]
    injected: Option<skate_host::bridge::Controls>,
    input: InputBuffer<InputFrame, (u16, u8)>,
    edges: VecDeque<Edge>,
    controller: Option<usize>,
    generation: u64,
    buttons: u16,
    flag_allowed: bool,
    flag_gesture: Gesture,
    flag_actions: VecDeque<(Instant, Action)>,
}

pub(super) struct Sampler {
    state: Arc<Mutex<State>>,
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    pub(super) fn spawn() -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let stopped = Arc::new(AtomicBool::new(false));
        let shared = state.clone();
        let stop = stopped.clone();
        let thread = std::thread::Builder::new()
            .name("skate-input".into())
            .spawn(move || {
                let mut transport = ControllerTransport::default();
                let mut deadline = Instant::now();
                let origin = deadline;
                while !stop.load(Ordering::Relaxed) {
                    #[cfg(feature = "dev")]
                    let injected = shared.lock().unwrap().injected;
                    #[cfg(feature = "dev")]
                    let frame =
                        injected.map_or_else(|| transport.poll(), InputFrame::from_controls);
                    #[cfg(not(feature = "dev"))]
                    let frame = transport.poll();
                    let now = Instant::now();
                    {
                        let mut state = shared.lock().unwrap();
                        let controller = frame.controller();
                        if controller != state.controller {
                            state.controller = controller;
                            state.generation = state.generation.wrapping_add(1);
                            state.input.clear();
                            state.edges.clear();
                            state.buttons = frame.buttons();
                            state.flag_gesture.cancel();
                            state.flag_actions.clear();
                        }
                        let held = frame.buttons();
                        let pressed = held & !state.buttons;
                        state.buttons = held;
                        let allowed = state.flag_allowed && controller.is_some();
                        if let Some(action) = state.flag_gesture.update(
                            now.duration_since(origin).as_millis() as u64,
                            held & 0x0020 != 0,
                            allowed,
                        ) {
                            if state.flag_actions.len() == 8 {
                                state.flag_actions.pop_front();
                            }
                            state.flag_actions.push_back((now, action));
                        }
                        if pressed != 0 {
                            if state.edges.len() == EDGE_CAPACITY {
                                state.edges.pop_front();
                            }
                            state.edges.push_back(Edge {
                                at: now,
                                pressed,
                                held,
                            });
                        }
                        let generation = state.generation;
                        let key = (held, frame.gesture_key());
                        state.input.push(
                            key,
                            Sample {
                                at: now,
                                generation,
                                value: frame.without_buttons(0x0020),
                            },
                        );
                    }
                    deadline += POLL_PERIOD;
                    let now = Instant::now();
                    if deadline <= now {
                        deadline = now + POLL_PERIOD;
                    }
                    std::thread::sleep(deadline.saturating_duration_since(now));
                }
            })?;
        Ok(Self {
            state,
            stopped,
            thread: Some(thread),
        })
    }

    pub(super) fn take(&self, now: Instant) -> (Option<Sample<InputFrame>>, u64, u64) {
        let mut state = self.state.lock().unwrap();
        (state.input.take(now), state.generation, state.input.dropped)
    }

    #[cfg(feature = "dev")]
    pub(super) fn inject(&self, controls: skate_host::bridge::Controls) {
        self.state.lock().unwrap().injected = Some(controls);
    }

    pub(super) fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.input.clear();
        state.edges.clear();
        state.generation = state.generation.wrapping_add(1);
        state.flag_gesture.cancel();
        state.flag_actions.clear();
    }

    pub(super) fn flag_actions(&self, allowed: bool, now: Instant) -> Vec<Action> {
        let mut state = self.state.lock().unwrap();
        if state.flag_allowed != allowed {
            state.flag_allowed = allowed;
            state.flag_gesture.cancel();
            state.flag_actions.clear();
        }
        state
            .flag_actions
            .drain(..)
            .filter(|(at, _)| allowed && now.saturating_duration_since(*at) <= FLAG_ACTION_AGE)
            .map(|(_, action)| action)
            .collect()
    }

    pub(super) fn edges(&self, now: Instant) -> (bool, Vec<Edge>) {
        let mut state = self.state.lock().unwrap();
        let edges = state
            .edges
            .drain(..)
            .filter(|e| now.saturating_duration_since(e.at) <= MAX_AGE)
            .collect();
        (state.controller.is_some(), edges)
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
