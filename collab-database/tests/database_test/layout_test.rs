use collab_database::fields::Field;
use collab_database::views::DatabaseLayout;

use crate::database_test::helper::{
  create_database_with_default_data, DatabaseTest, DatabaseTestBuilder,
};
use crate::helper::TestCalendarLayoutSetting;

#[test]
fn get_layout_setting_test() {
  let database_test = create_database_with_two_layout_settings();
  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Board)
    .unwrap();
  assert_eq!(layout_setting.field_id, "f1");

  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Grid)
    .unwrap();
  assert_eq!(layout_setting.field_id, "f2");
}

#[test]
fn create_database_view_with_layout_setting_test() {
  let database_test = DatabaseTestBuilder::new(1, "1")
    .with_layout(DatabaseLayout::Calendar)
    .with_field(Field::new(
      "f1".to_string(),
      "text field".to_string(),
      0,
      true,
    ))
    .with_layout_setting(TestCalendarLayoutSetting::new("f1".to_string()).into())
    .build();

  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Calendar)
    .unwrap();
  assert_eq!(layout_setting.field_id, "f1");
}

#[test]
fn remove_layout_setting_test() {
  let database_test = create_database_with_two_layout_settings();
  database_test.views.update_database_view("v1", |view| {
    view.remove_layout_setting(&DatabaseLayout::Board);
  });

  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Board);
  assert!(layout_setting.is_none());
}

#[test]
fn update_layout_setting_test() {
  let database_test = create_database_with_two_layout_settings();
  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Board)
    .unwrap();
  assert_eq!(layout_setting.first_day_of_week, 0);
  assert!(layout_setting.show_weekends);

  //
  let mut layout_setting = TestCalendarLayoutSetting::new("f1".to_string());
  layout_setting.show_weekends = false;
  layout_setting.first_day_of_week = 2;
  database_test.insert_layout_setting("v1", &DatabaseLayout::Board, layout_setting);

  //
  let layout_setting = database_test
    .views
    .get_layout_setting::<TestCalendarLayoutSetting>("v1", &DatabaseLayout::Board)
    .unwrap();
  assert_eq!(layout_setting.first_day_of_week, 2);
  assert!(!layout_setting.show_weekends);
}

fn create_database_with_two_layout_settings() -> DatabaseTest {
  let database_test = create_database_with_default_data(1, "1");
  let layout_setting_1 = TestCalendarLayoutSetting::new("f1".to_string());
  let layout_setting_2 = TestCalendarLayoutSetting::new("f2".to_string());

  database_test.insert_layout_setting("v1", &DatabaseLayout::Board, layout_setting_1);
  database_test.insert_layout_setting("v1", &DatabaseLayout::Grid, layout_setting_2);

  database_test
}
