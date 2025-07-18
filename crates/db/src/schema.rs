diesel::table! {
    collections (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        name -> Text,
        entries_table -> Text,
        schema -> Text,
    }
}
