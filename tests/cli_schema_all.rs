//! Every command's `data` is described in `bosun schema` (none left as `{..}`).

use bosun::schema;

#[test]
fn every_command_is_described() {
    for name in schema::names() {
        let d = schema::data_of(name).unwrap();
        assert_ne!(d.trim(), "{..}", "{name} is not described");
    }
}

#[test]
fn a_shared_shape_resolves_to_the_other_command() {
    // `verdict apply` has its own shape (a neutral refusal); `plan amend` shares `plan import`'s.
    assert_eq!(
        schema::data_of("plan amend"),
        schema::data_of("plan import")
    );
    assert!(schema::data_of("plan amend").unwrap().starts_with('{'));
    assert!(
        schema::data_of("verdict apply")
            .unwrap()
            .contains("refused")
    );
}
