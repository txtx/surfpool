diesel::table! {
    mcp (id) {
        id -> Integer,
        created_at -> Date,
        updated_at -> Date,
        name -> Text,
        description -> Text,
        command -> Text,
        version -> Text,
    }
}

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
