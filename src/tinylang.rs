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
