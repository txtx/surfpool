diesel::table! {
    collections (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        table_name -> Text,
        schema -> Text,
    }
}
