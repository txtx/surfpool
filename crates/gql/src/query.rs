use std::collections::HashMap;

use crate::Context;
use convert_case::{Case, Casing};
use juniper::{
    meta::{Field, MetaType},
    Arguments, BoxFuture, DefaultScalarValue, ExecutionResult, Executor, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct Query {}

#[derive(Clone, Debug)]
pub struct SchemaDatasource {
    pub entries: HashMap<String, SchemaDatasourceEntry>,
}

impl SchemaDatasource {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: SchemaDatasourceEntry) {
        self.entries.insert(entry.name.to_case(Case::Camel), entry);
    }
}

#[derive(Clone, Debug)]
pub struct SchemaDatasourceEntry {
    pub name: String,
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<String>,
}

impl SchemaDatasourceEntry {
    pub fn new(uuid: &Uuid, name: &str) -> Self {
        Self {
            name: name.to_case(Case::Pascal),
            subgraph_uuid: uuid.clone(),
            description: None,
            fields: vec![],
        }
    }
}

impl GraphQLType<DefaultScalarValue> for SchemaDatasourceEntry {
    fn name(spec: &SchemaDatasourceEntry) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &SchemaDatasourceEntry, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        fields.push(registry.field::<&String>("uuid", &()));
        for field in spec.fields.iter() {
            fields.push(registry.field::<&String>(field, &()));
        }
        registry
            .build_object_type::<SchemaDatasourceEntry>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SchemaDatasourceEntry {
    type Context = Context;
    type TypeInfo = SchemaDatasourceEntry;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <SchemaDatasourceEntry as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        info: &SchemaDatasourceEntry,
        field_name: &str,
        args: &Arguments,
        executor: &Executor<Context>,
    ) -> ExecutionResult {
        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        // let database = executor.context();
        // let subgraph_db = database.entries_store.read().unwrap();
        // let entries = subgraph_db.get(&self.subgraph_uuid).unwrap();

        // match args.get::<String>("uuid")? {
        //     Some(uuid) => unimplemented!(),
        //     None => {
        //         let results = entries
        //             .iter()
        //             .filter_map(|(_, e)| e.values.get(field_name).map(|e| e.to_string()))
        //             .collect::<Vec<_>>();
        //         println!("{:?}", results);
        //         unimplemented!()
        //         // executor.resolve(&info, &results)
        //     }
        // }
        unimplemented!()
    }
}

// impl GraphQLValueAsync<DefaultScalarValue> for SchemaDatasourceEntry {
//     fn resolve_field_async(
//         &self,
//         _info: &SchemaDatasourceEntry,
//         _field_name: &str,
//         _arguments: &Arguments,
//         _executor: &Executor<Context>,
//     ) -> BoxFuture<ExecutionResult> {
//         panic!(
//             "?",
//         );
//     }
// }

impl Query {
    pub fn new() -> Self {
        Self {}
    }
}

impl GraphQLType<DefaultScalarValue> for Query {
    fn name(_spec: &SchemaDatasource) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(spec: &SchemaDatasource, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        // First, we need to define all fields and their types on this type.
        //
        // If we need arguments, want to implement interfaces, or want to add documentation
        // strings, we can do it here.
        let mut fields = vec![];
        fields.push(registry.field::<&String>("id", &()));
        fields.push(registry.field::<&String>("name", &()));
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, entry) in spec.entries.iter() {
            fields.push(registry.field::<&[SchemaDatasourceEntry]>(name, &entry));
        }
        registry
            .build_object_type::<Query>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for Query {
    type Context = Context;
    type TypeInfo = SchemaDatasource;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Query as GraphQLType>::name(&info)
    }

    fn resolve_field(
        &self,
        info: &SchemaDatasource,
        subgraph_name: &str,
        args: &Arguments,
        executor: &Executor<Self::Context>,
    ) -> ExecutionResult {
        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        // let database = executor.context();
        // let subgraph_db = database.entries_store.read().unwrap();
        // let entries = subgraph_db.get(&self.subgraph_uuid).unwrap();

        // match args.get::<String>("uuid")? {
        //     Some(uuid) => {
        //         let results = entries
        //             .iter()
        //             .filter_map(|(_, e)| e.values.get(field_name).map(|e| e.to_string()))
        //             .collect::<Vec<_>>();
        //         println!("{:?}", results);
        //         unimplemented!()
        //         executor.resolve(&info, &results)
        //     },
        //     None => unimplemented!()
        // }

        unimplemented!();

        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        let database = executor.context();

        // match subgraph_name {
        // Because scalars are defined with another `Context` associated type, you must use
        // `resolve_with_ctx` here to make the `executor` perform automatic type conversion
        // of its argument.
        // "api_version" => executor.resolve_with_ctx(&(), "1.0"),
        // subgraph => executor.resolve_with_ctx(&(), &self.name),
        // You pass a vector of `User` objects to `executor.resolve`, and it will determine
        // which fields of the sub-objects to actually resolve based on the query.
        // The `executor` instance keeps track of its current position in the query.
        // "friends" => executor.resolve(info,
        //     &self.friend_ids.iter()
        //         .filter_map(|id| database.users.get(id))
        //         .collect::<Vec<_>>()
        // ),
        // We can only reach this panic in two cases: either a mismatch between the defined
        // schema in `meta()` above, or a validation failed because of a this library bug.
        //
        // In either of those two cases, the only reasonable way out is to panic the thread.
        // _ => {
        //     for (name, entry) in info.entries.iter() {
        //         if name.eq(subgraph_name) {
        //             return executor.resolve(database, entry)
        //         }
        //     }
        //     panic!("unable to resolve")

        // }
        // }
    }
}

// impl GraphQLValueAsync<DefaultScalarValue> for Query {}

impl GraphQLValueAsync<DefaultScalarValue> for Query {
    fn resolve_field_async(
        &self,
        info: &SchemaDatasource,
        field_name: &str,
        _arguments: &Arguments,
        executor: &Executor<Context>,
    ) -> BoxFuture<ExecutionResult> {
        println!("{}", field_name);
        println!("{:?}", _arguments);

        let res = match field_name {
            "apiVersion" => executor.resolve_with_ctx(&(), "1.0"),
            subgraph_name => {
                let database = executor.context();
                let subgraph_db = database.entries_store.read().unwrap();
                let entry = info.entries.get(subgraph_name).unwrap();
                let (_, entries) = subgraph_db.get(subgraph_name).unwrap();
                executor.resolve_with_ctx(entry, &entries[..])
            }
        };

        Box::pin(async { res })
    }
}
