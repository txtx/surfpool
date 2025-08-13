diesel::table! {
    collections (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        table_name -> Text,
        workspace_slug -> Text,
        schema -> Text,
        last_slot_processed -> Int4,
        worker_id -> Text,
    }
}

diesel::table! {
    workers (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        cursor -> Nullable<Text>,
        token -> Text,
        last_slot_processed -> Int4,
    }
}
