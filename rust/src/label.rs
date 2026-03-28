pub(crate) fn label_from_engine(engine_name: &str) -> &'static str {
    let lower = engine_name.to_lowercase();
    for keyword in &["mozc", "anthy", "kkc", "japanese", "kana"] {
        if lower.contains(keyword) {
            return "\u{3042}";
        }
    }
    "A"
}

pub(crate) fn label_from_symbol(symbol: &str) -> &'static str {
    match symbol {
        "\u{3042}" | "\u{30A2}" | "\u{FF71}" => "\u{3042}",
        "A" | "_" => "A",
        _ => {
            let lower = symbol.to_lowercase();
            if lower.contains("hiragana") || lower.contains("katakana") {
                "\u{3042}"
            } else if lower.contains("latin")
                || lower.contains("direct")
                || lower.contains("alphanumeric")
            {
                "A"
            } else {
                "A"
            }
        }
    }
}
