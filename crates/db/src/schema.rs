diesel::table! {
    collections (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        subgraph_id -> Text,
        entries_table -> Text,
        schema -> Text,
    }
}
