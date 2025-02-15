use juniper::{
    meta::MetaType, Arguments, Context, DefaultScalarValue, ExecutionResult, Executor, GraphQLType,
    GraphQLValue, Registry,
};
use std::collections::HashMap;

pub struct SubgraphSpecification {
    name: String,
}

#[derive(Debug)]
pub struct Database {
    subgraphs: HashMap<String, Subgraph>,
}
impl Context for Database {}

#[derive(Debug)]
pub struct Subgraph {
    id: String,
    name: String,
    // friend_ids: Vec<String>
}

impl GraphQLType<DefaultScalarValue> for Subgraph {
    fn name(spec: &SubgraphSpecification) -> Option<&str> {
        Some(&spec.name)
    }

    fn meta<'r>(spec: &SubgraphSpecification, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        // First, we need to define all fields and their types on this type.
        //
        // If we need arguments, want to implement interfaces, or want to add documentation
        // strings, we can do it here.
        let fields = &[
            registry.field::<&String>("id", &()),
            registry.field::<&String>("name", &()),
            //    registry.field::<Vec<&Subgraph>>("friends", &spec),
        ];
        registry
            .build_object_type::<Subgraph>(&spec, fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for Subgraph {
    type Context = Database;
    type TypeInfo = SubgraphSpecification;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Subgraph as GraphQLType>::name(&info)
    }

    fn resolve_field(
        &self,
        info: &SubgraphSpecification,
        field_name: &str,
        args: &Arguments,
        executor: &Executor<Database>,
    ) -> ExecutionResult {
        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        let database = executor.context();
        match field_name {
            // Because scalars are defined with another `Context` associated type, you must use
            // `resolve_with_ctx` here to make the `executor` perform automatic type conversion
            // of its argument.
            "id" => executor.resolve_with_ctx(&(), &self.id),
            "name" => executor.resolve_with_ctx(&(), &self.name),
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
            _ => panic!("Field {field_name} not found on type User"),
        }
    }
}
