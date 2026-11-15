//! `cards.map` (which rectangle of which photo is which card) must agree with
//! the card data, so `scripts/crop_cards.sh` cuts the right picture for every
//! id.

use gamelib::id::DefId;
use gamelib_res_arcana::rules::ResArcanaRules;

const MAP: &str = include_str!("../cards.map");

/// Size of the photos, as `identify` reports them (the rectangles must lie
/// inside).
fn photo_size(source: &str) -> (u32, u32) {
    match source {
        "all_cards.jpg" => (3958, 2677),
        "components.jpg" => (2911, 2572),
        other => panic!("unknown photo {other}"),
    }
}

struct Row {
    id:     u32,
    name:   String,
    source: String,
    x:      u32,
    y:      u32,
    w:      u32,
    h:      u32,
}

fn rows() -> Vec<Row> {
    MAP.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|line| {
            let f: Vec<_> = line.split('\t').collect();
            assert_eq!(f.len(), 7, "seven tab-separated columns: {line:?}");
            let n =
                |i: usize| f[i].parse::<u32>().unwrap_or_else(|_| panic!("a number in {line:?}"));
            Row {
                id:     n(0),
                name:   f[1].to_owned(),
                source: f[2].to_owned(),
                x:      n(3),
                y:      n(4),
                w:      n(5),
                h:      n(6),
            }
        })
        .collect()
}

#[test]
fn every_base_card_has_exactly_one_row_with_its_printed_name() {
    let rules = ResArcanaRules::new();
    let total = rules.cards().names.len();
    let rows = rows();
    assert_eq!(rows.len(), total, "one row per card");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            usize::try_from(row.id).unwrap(),
            i,
            "rows are sorted by id with no gaps"
        );
        let spec = rules.spec(DefId(row.id)).expect("a card with this id");
        assert_eq!(row.name, spec.name, "id {} is {}", row.id, spec.name);
    }
}

#[test]
fn every_rectangle_lies_inside_its_photo_and_has_a_card_shape() {
    for row in rows() {
        let (width, height) = photo_size(&row.source);
        assert!(
            row.w > 100 && row.h > 100,
            "{}: a card is not a speck",
            row.name
        );
        assert!(
            row.x + row.w <= width && row.y + row.h <= height,
            "{}: inside the photo",
            row.name
        );
        let ratio = f64::from(row.h) / f64::from(row.w);
        assert!(
            (1.2..1.6).contains(&ratio),
            "{}: portrait card proportions, got {ratio:.2}",
            row.name
        );
    }
}

/// The photo is shot in perspective and the rectangles are read by eye (good to
/// about 5-10 px), so neighbours may share a sliver; two cards really colliding
/// would overlap by far more.
const SLIVER: u32 = 12;

#[test]
fn cards_cut_from_one_photo_do_not_overlap_by_more_than_a_sliver() {
    let rows = rows();
    for source in ["all_cards.jpg", "components.jpg"] {
        let of: Vec<_> = rows.iter().filter(|r| r.source == source).collect();
        for a in &of {
            for b in &of {
                if a.id < b.id {
                    let across = (a.x + a.w).min(b.x + b.w).saturating_sub(a.x.max(b.x));
                    let down = (a.y + a.h).min(b.y + b.h).saturating_sub(a.y.max(b.y));
                    assert!(
                        across <= SLIVER || down <= SLIVER,
                        "{} and {} overlap by {across}x{down} px in {source}",
                        a.name,
                        b.name
                    );
                }
            }
        }
    }
}
