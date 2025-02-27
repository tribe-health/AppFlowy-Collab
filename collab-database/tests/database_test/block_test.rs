use collab_database::rows::CreateRowParams;

use crate::database_test::helper::create_database;

#[test]
fn create_rows_test() {
  let database_test = create_database(1, "1");
  for i in 0..100 {
    database_test.create_row_in_view(
      "v1",
      CreateRowParams {
        id: i.into(),
        ..Default::default()
      },
    );
  }
  let rows = database_test.get_rows_for_view("v1");
  assert_eq!(rows.len(), 100);
}
