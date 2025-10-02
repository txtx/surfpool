diesel::table! {
    collections (id) {
        id -> Text,
        created_at -> Date,
        updated_at -> Date,
        table_name -> Text,
        workspace_slug -> Text,
        source -> Text,
        latest_slot_successfully_processed -> Int4,
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

diesel::table! {
    svm_subgraph_revisions (id) {
        id -> Text,
        source -> Text,
        latest_slot_successfully_processed -> Int4,
        worker_id -> Text,
        workspace_id -> Text,
    }
}

diesel::table! {
    svm_workspaces (id) {
        id -> Text,
        name -> Text,
        slug -> Text,
    }
}
