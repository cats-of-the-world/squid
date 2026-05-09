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
        TinyLangType::Object(o) => o.get(key).unwrap().to_string(),
        _ => panic!("vector is not a vector of objects"),
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
