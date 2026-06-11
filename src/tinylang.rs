use std::fs;
use tinylang::eval;
use tinylang::types::{FuncArguments, State, TinyLangType};

/// exposes render as a function in the template itself.
pub fn render(arguments: FuncArguments, state: &State) -> TinyLangType {
    if arguments.is_empty() {
        return TinyLangType::Nil;
    }

    let page = match arguments.first().unwrap() {
        TinyLangType::String(page) => page.as_str(),
        _ => return TinyLangType::Nil,
    };

    let result = match fs::read_to_string(page) {
        Ok(c) => eval(&c, state.clone()),
        Err(e) => return TinyLangType::String(e.to_string()),
    };

    match result {
        Ok(content) => TinyLangType::String(content),
        Err(e) => TinyLangType::String(e.to_string()),
    }
}

/// sort array of objects by a key
pub fn sort_by_key(arguments: FuncArguments, _state: &State) -> TinyLangType {
    if arguments.len() < 2 {
        return TinyLangType::Nil;
    }

    let mut collection = match arguments.first() {
        Some(TinyLangType::Vec(vec)) => vec.clone(),
        _ => return TinyLangType::Nil,
    };

    let key = match arguments.get(1) {
        Some(TinyLangType::String(s)) => s,
        _ => return TinyLangType::Nil,
    };

    collection.sort_by_key(|e| match e {
        TinyLangType::Object(o) => o.get(key).map(|v| v.to_string()).unwrap_or_default(),
        _ => String::new(),
    });

    if arguments.len() == 3
        && &TinyLangType::String("reversed".to_string()) == arguments.get(2).unwrap()
    {
        collection.reverse();
    }

    TinyLangType::Vec(collection)
}

/// return a slice of items for a given 1-based page number and page size
pub fn paginate(arguments: FuncArguments, _state: &State) -> TinyLangType {
    if arguments.len() < 3 {
        return TinyLangType::Nil;
    }

    let items = match arguments.first() {
        Some(TinyLangType::Vec(v)) => v.clone(),
        _ => return TinyLangType::Nil,
    };

    let page = match arguments.get(1) {
        Some(TinyLangType::Numeric(n)) => *n as usize,
        _ => return TinyLangType::Nil,
    };

    let per_page = match arguments.get(2) {
        Some(TinyLangType::Numeric(n)) => *n as usize,
        _ => return TinyLangType::Nil,
    };

    if page == 0 || per_page == 0 {
        return TinyLangType::Nil;
    }

    let start = (page - 1) * per_page;
    if start >= items.len() {
        return TinyLangType::Vec(Vec::new());
    }
    let end = (start + per_page).min(items.len());
    TinyLangType::Vec(items[start..end].to_vec())
}

/// format a frontmatter date with a chrono strftime string:
/// date_format(post.date, '%B %d, %Y') -> "January 10, 2024"
pub fn date_format(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let (Some(TinyLangType::String(date)), Some(TinyLangType::String(fmt))) =
        (arguments.first(), arguments.get(1))
    else {
        return TinyLangType::Nil;
    };

    let Some(parsed) = crate::md::parse_date(date) else {
        return TinyLangType::Nil;
    };

    // validate the format string first: chrono panics when displaying a
    // DelayedFormat built from an invalid specifier
    let items: Vec<chrono::format::Item> = chrono::format::StrftimeItems::new(fmt).collect();
    if items
        .iter()
        .any(|i| matches!(i, chrono::format::Item::Error))
    {
        return TinyLangType::Nil;
    }

    TinyLangType::String(parsed.format_with_items(items.into_iter()).to_string())
}

/// turn any string into a url-friendly slug: slugify('Rust & Web') -> "rust-web"
pub fn slugify(arguments: FuncArguments, _state: &State) -> TinyLangType {
    match arguments.first() {
        Some(TinyLangType::String(s)) => TinyLangType::String(crate::tags::slugify(s)),
        _ => TinyLangType::Nil,
    }
}

/// filter objects by a key's value: where(posts.items, 'author', 'era')
pub fn where_fn(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let (Some(TinyLangType::Vec(items)), Some(TinyLangType::String(key)), Some(value)) =
        (arguments.first(), arguments.get(1), arguments.get(2))
    else {
        return TinyLangType::Nil;
    };

    let expected = value.to_string();
    TinyLangType::Vec(
        items
            .iter()
            .filter(|item| match item {
                TinyLangType::Object(o) => o
                    .get(key)
                    .map(|v| v.to_string() == expected)
                    .unwrap_or(false),
                _ => false,
            })
            .cloned()
            .collect(),
    )
}

/// take the first n items of an array: limit(posts.items, 5)
pub fn limit(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let (Some(TinyLangType::Vec(items)), Some(TinyLangType::Numeric(n))) =
        (arguments.first(), arguments.get(1))
    else {
        return TinyLangType::Nil;
    };

    let n = (*n).max(0.0) as usize;
    TinyLangType::Vec(items.iter().take(n).cloned().collect())
}

/// group objects by a key's value: group_by(posts.items, 'author') returns
/// [{key: 'era', items: [...]}, ...] sorted by key
pub fn group_by(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let (Some(TinyLangType::Vec(items)), Some(TinyLangType::String(key))) =
        (arguments.first(), arguments.get(1))
    else {
        return TinyLangType::Nil;
    };

    let mut groups: std::collections::BTreeMap<String, Vec<TinyLangType>> =
        std::collections::BTreeMap::new();
    for item in items {
        let group_key = match item {
            TinyLangType::Object(o) => o.get(key).map(|v| v.to_string()).unwrap_or_default(),
            _ => String::new(),
        };
        groups.entry(group_key).or_default().push(item.clone());
    }

    TinyLangType::Vec(
        groups
            .into_iter()
            .map(|(group_key, group_items)| {
                let mut group = State::new();
                group.insert("key".into(), group_key.into());
                group.insert("items".into(), TinyLangType::Vec(group_items));
                TinyLangType::Object(group)
            })
            .collect(),
    )
}

/// shorten a string to at most n characters, appending an ellipsis when cut:
/// truncate(post.title, 50)
pub fn truncate(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let (Some(TinyLangType::String(s)), Some(TinyLangType::Numeric(n))) =
        (arguments.first(), arguments.get(1))
    else {
        return TinyLangType::Nil;
    };

    let n = (*n).max(0.0) as usize;
    if s.chars().count() <= n {
        return TinyLangType::String(s.clone());
    }
    let cut: String = s.chars().take(n).collect();
    TinyLangType::String(format!("{}…", cut.trim_end()))
}

/// reverse an array
pub fn reverse(arguments: FuncArguments, _state: &State) -> TinyLangType {
    let mut collection = match arguments.first() {
        Some(TinyLangType::Vec(vec)) => vec.clone(),
        _ => return TinyLangType::Nil,
    };

    collection.reverse();

    TinyLangType::Vec(collection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_obj(key: &str, val: &str) -> TinyLangType {
        let mut state = State::new();
        state.insert(key.to_string(), TinyLangType::String(val.to_string()));
        TinyLangType::Object(state)
    }

    fn extract_str(item: &TinyLangType, key: &str) -> String {
        if let TinyLangType::Object(o) = item {
            if let Some(TinyLangType::String(s)) = o.get(key) {
                return s.clone();
            }
        }
        panic!("expected string at key {key}");
    }

    #[test]
    fn test_sort_by_key_basic() {
        let collection = TinyLangType::Vec(vec![
            make_obj("name", "Charlie"),
            make_obj("name", "Alice"),
            make_obj("name", "Bob"),
        ]);
        let result = sort_by_key(
            vec![collection, TinyLangType::String("name".into())],
            &State::new(),
        );
        let TinyLangType::Vec(items) = result else {
            panic!("expected vec");
        };
        let names: Vec<String> = items.iter().map(|i| extract_str(i, "name")).collect();
        assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_sort_by_key_reversed() {
        let collection = TinyLangType::Vec(vec![
            make_obj("name", "Alice"),
            make_obj("name", "Charlie"),
            make_obj("name", "Bob"),
        ]);
        let result = sort_by_key(
            vec![
                collection,
                TinyLangType::String("name".into()),
                TinyLangType::String("reversed".into()),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(items) = result else {
            panic!("expected vec");
        };
        let names: Vec<String> = items.iter().map(|i| extract_str(i, "name")).collect();
        assert_eq!(names, vec!["Charlie", "Bob", "Alice"]);
    }

    #[test]
    fn test_sort_by_key_insufficient_args() {
        let result = sort_by_key(vec![TinyLangType::String("x".into())], &State::new());
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_sort_by_key_wrong_type() {
        let result = sort_by_key(
            vec![TinyLangType::Numeric(1.0), TinyLangType::String("k".into())],
            &State::new(),
        );
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_paginate_basic() {
        let items = TinyLangType::Vec((1..=10).map(|i| TinyLangType::Numeric(i as f64)).collect());
        let result = paginate(
            vec![
                items,
                TinyLangType::Numeric(2.0),
                TinyLangType::Numeric(3.0),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(page) = result else {
            panic!("expected vec");
        };
        assert_eq!(page.len(), 3);
        assert!(page[0] == TinyLangType::Numeric(4.0));
        assert!(page[2] == TinyLangType::Numeric(6.0));
    }

    #[test]
    fn test_paginate_last_page_partial() {
        let items = TinyLangType::Vec((1..=5).map(|i| TinyLangType::Numeric(i as f64)).collect());
        let result = paginate(
            vec![
                items,
                TinyLangType::Numeric(2.0),
                TinyLangType::Numeric(3.0),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(page) = result else {
            panic!("expected vec");
        };
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn test_paginate_page_out_of_bounds() {
        let items = TinyLangType::Vec((1..=3).map(|i| TinyLangType::Numeric(i as f64)).collect());
        let result = paginate(
            vec![
                items,
                TinyLangType::Numeric(5.0),
                TinyLangType::Numeric(3.0),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(page) = result else {
            panic!("expected empty vec, got non-vec");
        };
        assert!(page.is_empty());
    }

    #[test]
    fn test_paginate_zero_page_returns_nil() {
        let items = TinyLangType::Vec(vec![TinyLangType::Numeric(1.0)]);
        let result = paginate(
            vec![
                items,
                TinyLangType::Numeric(0.0),
                TinyLangType::Numeric(3.0),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_paginate_insufficient_args() {
        let result = paginate(vec![TinyLangType::Numeric(1.0)], &State::new());
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_date_format_basic() {
        let result = date_format(
            vec![
                TinyLangType::String("2024-01-10".into()),
                TinyLangType::String("%B %d, %Y".into()),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::String("January 10, 2024".into()));
    }

    #[test]
    fn test_date_format_invalid_date_returns_nil() {
        let result = date_format(
            vec![
                TinyLangType::String("not a date".into()),
                TinyLangType::String("%Y".into()),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_date_format_invalid_format_returns_nil() {
        let result = date_format(
            vec![
                TinyLangType::String("2024-01-10".into()),
                TinyLangType::String("%QQQ".into()),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::Nil);
    }

    #[test]
    fn test_slugify_function() {
        let result = slugify(
            vec![TinyLangType::String("Hello, World!".into())],
            &State::new(),
        );
        assert!(result == TinyLangType::String("hello-world".into()));
    }

    #[test]
    fn test_where_filters_by_value() {
        let items = TinyLangType::Vec(vec![
            make_obj("author", "era"),
            make_obj("author", "other"),
            make_obj("author", "era"),
        ]);
        let result = where_fn(
            vec![
                items,
                TinyLangType::String("author".into()),
                TinyLangType::String("era".into()),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(filtered) = result else {
            panic!("expected vec");
        };
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_where_missing_key_excluded() {
        let items = TinyLangType::Vec(vec![make_obj("author", "era"), make_obj("title", "x")]);
        let result = where_fn(
            vec![
                items,
                TinyLangType::String("author".into()),
                TinyLangType::String("era".into()),
            ],
            &State::new(),
        );
        let TinyLangType::Vec(filtered) = result else {
            panic!("expected vec");
        };
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_limit_takes_first_n() {
        let items = TinyLangType::Vec((1..=5).map(|i| TinyLangType::Numeric(i as f64)).collect());
        let result = limit(vec![items, TinyLangType::Numeric(2.0)], &State::new());
        let TinyLangType::Vec(limited) = result else {
            panic!("expected vec");
        };
        assert_eq!(limited.len(), 2);
        assert!(limited[0] == TinyLangType::Numeric(1.0));
    }

    #[test]
    fn test_limit_larger_than_len() {
        let items = TinyLangType::Vec(vec![TinyLangType::Numeric(1.0)]);
        let result = limit(vec![items, TinyLangType::Numeric(10.0)], &State::new());
        let TinyLangType::Vec(limited) = result else {
            panic!("expected vec");
        };
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn test_group_by_groups_and_sorts() {
        let items = TinyLangType::Vec(vec![
            make_obj("author", "zoe"),
            make_obj("author", "anna"),
            make_obj("author", "zoe"),
        ]);
        let result = group_by(
            vec![items, TinyLangType::String("author".into())],
            &State::new(),
        );
        let TinyLangType::Vec(groups) = result else {
            panic!("expected vec");
        };
        assert_eq!(groups.len(), 2);
        let TinyLangType::Object(first) = &groups[0] else {
            panic!("expected object");
        };
        assert!(first.get("key").unwrap() == &TinyLangType::String("anna".into()));
        let TinyLangType::Object(second) = &groups[1] else {
            panic!("expected object");
        };
        let Some(TinyLangType::Vec(zoe_items)) = second.get("items") else {
            panic!("expected items vec");
        };
        assert_eq!(zoe_items.len(), 2);
    }

    #[test]
    fn test_truncate_shortens_with_ellipsis() {
        let result = truncate(
            vec![
                TinyLangType::String("hello world".into()),
                TinyLangType::Numeric(5.0),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::String("hello…".into()));
    }

    #[test]
    fn test_truncate_short_string_unchanged() {
        let result = truncate(
            vec![
                TinyLangType::String("hi".into()),
                TinyLangType::Numeric(5.0),
            ],
            &State::new(),
        );
        assert!(result == TinyLangType::String("hi".into()));
    }

    #[test]
    fn test_reverse_basic() {
        let items = TinyLangType::Vec(vec![
            TinyLangType::Numeric(1.0),
            TinyLangType::Numeric(2.0),
            TinyLangType::Numeric(3.0),
        ]);
        let TinyLangType::Vec(result) = reverse(vec![items], &State::new()) else {
            panic!("expected vec");
        };
        assert_eq!(result.len(), 3);
        assert!(result[0] == TinyLangType::Numeric(3.0));
        assert!(result[1] == TinyLangType::Numeric(2.0));
        assert!(result[2] == TinyLangType::Numeric(1.0));
    }

    #[test]
    fn test_reverse_empty() {
        let items = TinyLangType::Vec(vec![]);
        let TinyLangType::Vec(result) = reverse(vec![items], &State::new()) else {
            panic!("expected vec");
        };
        assert!(result.is_empty());
    }

    #[test]
    fn test_reverse_wrong_type() {
        let result = reverse(vec![TinyLangType::String("x".into())], &State::new());
        assert!(result == TinyLangType::Nil);
    }
}
