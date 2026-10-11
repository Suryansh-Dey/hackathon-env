//! Plain-language scrap vocabulary → taxonomy IDs. Used by the offline mock
//! responders; phrase order matters (more specific phrases first).

use regex::Regex;
use std::sync::OnceLock;

pub const KEYWORDS: &[(&[&str], &str)] = &[
    (&["crt monitor"], "ewaste.itew.monitor_crt"),
    (&["crt tv", "crt television", "old tv", "picture tube"], "ewaste.ceew.tv_crt"),
    (&["window ac", "window air conditioner"], "ewaste.ceew.window_ac"),
    (&["indoor unit"], "ewaste.ceew.split_ac_indoor"),
    (&["split ac", "outdoor unit", "air conditioners", "air conditioner", "acs", "ac"], "ewaste.ceew.split_ac_outdoor"),
    (&["washing machines", "washing machine"], "ewaste.ceew.washing_machine"),
    (&["refrigerators", "refrigerator", "fridges", "fridge"], "ewaste.ceew.refrigerator"),
    (&["microwave oven", "microwave"], "ewaste.ceew.microwave"),
    (&["tube lights", "tube light", "tubelights", "tubelight", "cfls", "cfl", "bulbs", "bulb"], "ewaste.ceew.cfl_tube"),
    (&["ceiling fans", "ceiling fan", "fans", "fan"], "ewaste.lseew.ceiling_fan"),
    (&["televisions", "television", "led tv", "lcd tv", "tvs", "tv"], "ewaste.ceew.tv_flat"),
    (&["laptops", "laptop"], "ewaste.itew.laptop"),
    (&["cpu cabinet", "desktop computer", "desktops", "desktop", "computers", "computer"], "ewaste.itew.desktop_cpu"),
    (&["monitors", "monitor", "lcd screen"], "ewaste.itew.monitor_lcd"),
    (&["mobile phones", "mobile phone", "smartphones", "smartphone", "mobiles", "mobile", "phones", "phone"], "ewaste.itew.mobile_phone"),
    (&["printers", "printer"], "ewaste.itew.printer"),
    (&["keyboards", "keyboard", "mouse", "routers", "router", "chargers", "charger"], "ewaste.itew.small_mixed"),
    (&["inverter batteries", "inverter battery", "car batteries", "car battery", "lead acid", "lead-acid"], "battery.lead_acid"),
    (&["power banks", "power bank", "powerbank", "lithium", "li-ion"], "battery.li_ion"),
    (&["batteries", "battery", "cells"], "battery.small_mixed"),
    (&["copper"], "metal.copper"),
    (&["brass"], "metal.brass"),
    (&["aluminium", "aluminum"], "metal.aluminium"),
    (&["stainless steel", "stainless"], "metal.stainless_steel"),
    (&["iron", "steel", "almirah", "rods"], "metal.iron_steel"),
    (&["cables", "cable", "wires", "wire"], "ewaste.itew.cables"),
    (&["newspapers", "newspaper", "news paper", "raddi", "akhbar"], "paper.newspaper"),
    (&["cardboard", "cartons", "carton", "boxes", "box"], "paper.cardboard"),
    (&["magazines", "magazine", "books", "book", "notebooks"], "paper.books_magazines"),
    (&["office paper", "white paper", "printouts", "a4"], "paper.office_white"),
    (&["pet bottles", "pet bottle", "plastic bottles", "plastic bottle", "water bottles"], "plastic.pet_bottle"),
    (&["buckets", "bucket", "jerry cans", "hdpe"], "plastic.hdpe"),
    (&["plastic bags", "polythene", "plastic film"], "plastic.soft_film"),
    (&["mixed plastic", "hard plastic", "plastic chairs", "plastic toys", "plastic"], "plastic.hard_mixed"),
    (&["glass bottles", "beer bottles"], "glass.bottles"),
    (&["glass"], "glass.mixed"),
    (&["old clothes", "clothes", "textiles", "textile", "cloth"], "textile.mixed"),
    (&["tyres", "tyre", "tires", "tire"], "rubber.tyre"),
    (&["mixed paper", "paper"], "paper.mixed"),
];

/// Bare family words a vendor might use in a rate card.
pub const FAMILIES: &[(&str, &str)] = &[
    ("metals", "metal"),
    ("metal", "metal"),
    ("papers", "paper"),
    ("paper", "paper"),
    ("plastics", "plastic"),
    ("plastic", "plastic"),
    ("glass", "glass"),
    ("e-waste", "ewaste"),
    ("ewaste", "ewaste"),
    ("electronics", "ewaste"),
    ("batteries", "battery"),
    ("textiles", "textile"),
    ("clothes", "textile"),
];

fn word_re(phrase: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(phrase))).unwrap()
}

fn compiled() -> &'static Vec<(Vec<Regex>, &'static str)> {
    static C: OnceLock<Vec<(Vec<Regex>, &'static str)>> = OnceLock::new();
    C.get_or_init(|| KEYWORDS.iter().map(|(ps, cat)| (ps.iter().map(|p| word_re(p)).collect(), *cat)).collect())
}

/// First category whose phrase appears in `text`, with the match position.
pub fn find_category(text: &str) -> Option<(&'static str, usize)> {
    for (res, cat) in compiled() {
        if let Some(m) = res.iter().filter_map(|r| r.find(text)).min_by_key(|m| m.start()) {
            return Some((cat, m.start()));
        }
    }
    None
}

/// Family for a bare family word (`metals` → `metal`).
pub fn find_family(text: &str) -> Option<&'static str> {
    let t = text.to_ascii_lowercase();
    FAMILIES.iter().find(|(w, _)| word_re(w).is_match(&t)).map(|(_, f)| *f)
}

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("a", 1),
    ("an", 1),
    ("single", 1),
    ("two", 2),
    ("pair", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("dozen", 12),
];

/// Quantity mentioned before position `before` in `clause` ("two old ACs" → 2).
pub fn quantity_before(clause: &str, before: usize) -> Option<u32> {
    let head = clause[..before.min(clause.len())].to_ascii_lowercase();
    for tok in head.split(|c: char| !c.is_ascii_alphanumeric()).rev().filter(|t| !t.is_empty()) {
        if let Ok(n) = tok.parse::<u32>() {
            return Some(n);
        }
        if let Some((_, n)) = NUMBER_WORDS.iter().find(|(w, _)| *w == tok) {
            return Some(*n);
        }
    }
    None
}

/// Splits free text into clauses on commas, semicolons, full stops, newlines and "and".
pub fn clauses(text: &str) -> Vec<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| Regex::new(r"(?i)[,;\n]|\.\s|\band\b|\bplus\b").unwrap());
    r.split(text).map(|s| s.trim().trim_end_matches('.').to_string()).filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_specific_phrases_first() {
        assert_eq!(find_category("two old ACs").unwrap().0, "ewaste.ceew.split_ac_outdoor");
        assert_eq!(find_category("an old CRT TV").unwrap().0, "ewaste.ceew.tv_crt");
        assert_eq!(find_category("copper wire").unwrap().0, "metal.copper");
        assert_eq!(find_category("inverter battery").unwrap().0, "battery.lead_acid");
        assert_eq!(find_category("some newspapers").unwrap().0, "paper.newspaper");
        assert!(find_category("a chair").is_none());
    }

    #[test]
    fn quantities_and_clauses() {
        let c = clauses("two old ACs, one fridge and some newspapers");
        assert_eq!(c, vec!["two old ACs", "one fridge", "some newspapers"]);
        assert_eq!(quantity_before("two old ACs", 8), Some(2));
        assert_eq!(quantity_before("3 laptops", 2), Some(3));
        assert_eq!(quantity_before("some newspapers", 5), None);
        assert_eq!(find_family("all metals +3%"), Some("metal"));
    }
}
