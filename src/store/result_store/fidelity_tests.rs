use super::harness_tests::*;
use super::*;

fn round_trip(cells: Vec<Cell>) -> StoredRow {
    let fx = fixture(1 << 30);
    let names: Vec<String> = (0..cells.len()).map(|index| format!("c{index}")).collect();
    let mut writer = fx
        .store
        .begin(ResultSpec {
            columns: names,
            ..spec(&["unused"])
        })
        .expect("begin");
    writer.push(&cells).expect("push");
    let handle = writer.finish(&complete()).expect("finish");
    read_all(&fx.store, handle.id).remove(0)
}

#[test]
fn result_store_fidelity_matrix_round_trips_every_storage_class_exactly() {
    let cells = vec![
        Cell::Null,
        Cell::Int(i64::MIN),
        Cell::Int(i64::MAX),
        Cell::Real(1.5),
        Cell::Real(-0.0),
        Cell::Text("nul\0inside".to_owned()),
        Cell::Text("four-byte \u{1F600} utf-8".to_owned()),
        Cell::Text(String::new()),
        Cell::Blob(vec![0, 1, 2, 255]),
        Cell::Blob(Vec::new()),
        Cell::Text("007".to_owned()),
        Cell::Text("1e3".to_owned()),
        Cell::Text("0x10".to_owned()),
    ];

    let row = round_trip(cells.clone());

    assert_eq!(row.cells, cells);
    assert!(row.nonfinite.is_empty());
}

#[test]
fn result_store_negative_zero_keeps_its_sign_bit() {
    let row = round_trip(vec![Cell::Real(-0.0)]);

    match row.cells[0] {
        Cell::Real(value) => assert!(value.is_sign_negative() && value == 0.0, "{value}"),
        ref other => panic!("expected a real, got {other:?}"),
    }
}

#[test]
fn result_store_nan_is_stored_as_null_and_marked() {
    let row = round_trip(vec![Cell::Int(1), Cell::Real(f64::NAN)]);

    assert_eq!(row.cells, vec![Cell::Int(1), Cell::Null]);
    assert_eq!(row.nonfinite, vec![(1, NonFinite::NaN)]);
}

#[test]
fn result_store_infinities_are_stored_natively_and_marked() {
    let row = round_trip(vec![
        Cell::Real(f64::INFINITY),
        Cell::Real(f64::NEG_INFINITY),
        Cell::Real(2.0),
    ]);

    assert_eq!(
        row.cells,
        vec![
            Cell::Real(f64::INFINITY),
            Cell::Real(f64::NEG_INFINITY),
            Cell::Real(2.0)
        ]
    );
    assert_eq!(
        row.nonfinite,
        vec![(0, NonFinite::Infinity), (1, NonFinite::NegInfinity)]
    );
}

#[test]
fn result_store_a_row_of_the_wrong_width_is_invalid_and_poisons_the_writer() {
    let fx = fixture(1 << 30);
    let mut writer = fx.store.begin(spec(&["a", "b"])).expect("begin");

    let wrong = writer.push(&[Cell::Null]);
    let after = writer.push(&[Cell::Null, Cell::Null]);

    assert!(
        matches!(wrong, Err(ResultStoreError::Invalid { .. })),
        "{wrong:?}"
    );
    assert!(
        matches!(after, Err(ResultStoreError::Invalid { .. })),
        "{after:?}"
    );
}

#[test]
fn result_store_a_result_without_columns_or_with_too_many_is_invalid() {
    let fx = fixture(1 << 30);
    let none = fx.store.begin(spec(&[]));
    let names: Vec<String> = (0..1001).map(|index| index.to_string()).collect();
    let many = fx.store.begin(ResultSpec {
        columns: names,
        ..spec(&["x"])
    });

    assert!(matches!(none, Err(ResultStoreError::Invalid { .. })));
    assert!(matches!(many, Err(ResultStoreError::Invalid { .. })));
}

#[derive(Debug, PartialEq)]
struct Pair {
    name: String,
    count: i64,
}

impl Materialize for Pair {
    fn columns() -> &'static [&'static str] {
        &["name", "count"]
    }

    fn to_values(&self) -> Vec<Cell> {
        vec![Cell::Text(self.name.clone()), Cell::Int(self.count)]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        match values {
            [Cell::Text(name), Cell::Int(count)] => Ok(Self {
                name: name.clone(),
                count: *count,
            }),
            _ => Err(ResultStoreError::Invalid {
                reason: "pair layout",
            }),
        }
    }
}

#[test]
fn result_store_typed_rows_round_trip_through_materialize_with_their_meta() {
    let fx = fixture(1 << 30);
    let rows: Vec<Pair> = (0..5)
        .map(|count| Pair {
            name: format!("n{count}"),
            count,
        })
        .collect();

    let handle = fx
        .store
        .write_all("leads", "proj", "rev-9", rows, &complete())
        .expect("write_all");
    let page = fx
        .store
        .read_typed::<Pair>(handle.id, 0, |_| Take::Yes)
        .expect("read_typed");

    assert_eq!(page.rows.len(), 5);
    assert_eq!(
        page.rows[3],
        Pair {
            name: "n3".to_owned(),
            count: 3
        }
    );
    assert_eq!(page.next_ord, None);
    assert_eq!(
        page.meta.columns,
        vec!["name".to_owned(), "count".to_owned()]
    );
    assert_eq!(
        (page.meta.tool.as_str(), page.meta.revision_id.as_str()),
        ("leads", "rev-9")
    );
    assert_eq!(page.meta.completeness, complete());
}

#[test]
fn result_store_typed_read_surfaces_a_layout_mismatch() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 3, 4).expect("write");

    let result = fx.store.read_typed::<Pair>(handle.id, 0, |_| Take::Yes);

    assert!(
        matches!(result, Err(ResultStoreError::Invalid { .. })),
        "{result:?}"
    );
}
