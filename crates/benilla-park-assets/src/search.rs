use super::Entry;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Relevance,
    Name,
    NameReverse,
    Category,
    Game,
    Height,
    Width,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Search {
    pub text: String,
    pub game: Option<String>,
    pub category: Option<String>,
    pub sort: Sort,
}
fn normalize(text: &str) -> String {
    let mut value = String::new();
    let mut space = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            value.push(c);
            space = false;
        } else if !space {
            value.push(' ');
            space = true;
        }
    }
    value.trim().to_owned()
}
fn terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                terms.push(normalize(&current));
                current.clear();
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        terms.push(normalize(&current));
    }
    terms.retain(|s| !s.is_empty());
    terms
}
pub fn search(entries: &[Entry], query: &Search) -> Vec<usize> {
    let words = terms(&query.text);
    let whole = normalize(&query.text);
    let mut hits = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if query.game.as_ref().is_some_and(|g| g != &e.game)
            || query.category.as_ref().is_some_and(|c| c != &e.category)
        {
            continue;
        }
        let name = normalize(&e.name);
        let all = normalize(&format!(
            "{} {} {} {} {:016x} {}",
            e.name,
            e.game,
            e.category,
            e.path,
            e.id,
            e.aliases.join(" ")
        ));
        if !words.iter().all(|s| all.contains(s)) {
            continue;
        }
        let score = if name == whole {
            0
        } else if name.starts_with(&whole) {
            1
        } else if words.iter().all(|s| name.contains(s)) {
            2
        } else {
            3
        };
        hits.push((i, score, name));
    }
    hits.sort_by(|a, b| {
        let ea = &entries[a.0];
        let eb = &entries[b.0];
        let axis =
            |e: &Entry, axis: usize| e.bounds.map_or(f32::INFINITY, |b| b[1][axis] - b[0][axis]);
        let order = match query.sort {
            Sort::Relevance => a.1.cmp(&b.1),
            Sort::Name => std::cmp::Ordering::Equal,
            Sort::NameReverse => b.2.cmp(&a.2),
            Sort::Category => ea.category.cmp(&eb.category),
            Sort::Game => ea.game.cmp(&eb.game),
            Sort::Height => axis(ea, 1).total_cmp(&axis(eb, 1)),
            Sort::Width => axis(ea, 0).total_cmp(&axis(eb, 0)),
        };
        order
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| ea.id.cmp(&eb.id))
    });
    hits.into_iter().map(|h| h.0).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(name: &str, game: &str, path: &str) -> Entry {
        Entry {
            id: 1,
            game: game.into(),
            name: name.into(),
            category: "Ramps".into(),
            path: path.into(),
            aliases: vec!["wooden skate obstacle".into()],
            package: None,
            bounds: None,
        }
    }
    #[test]
    fn search_matches_names_aliases_paths_phrases_and_game_without_case() {
        let list = vec![
            entry("Large Quarter Pipe", "Skate 3", "park/quarter_pipe.rx2"),
            entry("Wooden Ramp", "WoW", "world/test.m2"),
        ];
        for term in [
            "QUARTER",
            "quarter pipe",
            "\"quarter pipe\" large",
            "park/quarter_pipe",
            "large wooden",
        ] {
            assert_eq!(
                search(
                    &list,
                    &Search {
                        text: term.into(),
                        ..Default::default()
                    }
                ),
                vec![0],
                "{term}"
            );
        }
        assert_eq!(
            search(
                &list,
                &Search {
                    text: "wood".into(),
                    game: Some("WoW".into()),
                    ..Default::default()
                }
            ),
            vec![1]
        );
        assert!(search(
            &list,
            &Search {
                text: "missing".into(),
                ..Default::default()
            }
        )
        .is_empty());
    }
}
