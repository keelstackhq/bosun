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
    assert_eq!(
        schema::data_of("verdict apply"),
        schema::data_of("verdict post")
    );
    assert!(schema::data_of("plan amend").unwrap().starts_with('{'));
}
