use crate::cli::Context;
use actix_cors::Cors;
use actix_web::dev::ServerHandle;
use actix_web::http::header::{self};
use actix_web::web::{self, Data};
use actix_web::HttpRequest;
use actix_web::{middleware, App, HttpResponse, HttpServer};
use actix_web::{Error, Responder};
use crossbeam::channel::{Receiver, Select, Sender};
use juniper_actix::{graphiql_handler, graphql_handler, playground_handler, subscriptions};
use juniper_graphql_ws::ConnectionConfig;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::sync::RwLock;
use std::thread::JoinHandle;
use std::time::Duration;
use surfpool_gql::query::SchemaDataSource;
use surfpool_gql::types::schema::DynamicSchemaMetadata;
use surfpool_gql::types::GqlSubgraphDataEntry;
use surfpool_gql::{new_dynamic_schema, Context as GqlContext, GqlDynamicSchema};
use surfpool_types::{
    SchemaDataSourcingEvent, SubgraphCommand, SubgraphDataEntry, SubgraphEvent, SurfpoolConfig,
};
use txtx_core::kit::types::types::Value;

#[cfg(feature = "explorer")]
use rust_embed::RustEmbed;

#[cfg(feature = "explorer")]
#[derive(RustEmbed)]
#[folder = "../../../explorer/.next/server/app"]
pub struct Asset;

pub async fn start_subgraph_and_explorer_server(
    network_binding: String,
    _config: SurfpoolConfig,
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    _ctx: &Context,
) -> Result<(ServerHandle, JoinHandle<Result<(), String>>), Box<dyn StdError>> {
    let context = GqlContext::new();
    let schema_datasource = SchemaDataSource::new();
    let schema = RwLock::new(Some(new_dynamic_schema(schema_datasource.clone())));
    let schema_wrapped = Data::new(schema);
    let context_wrapped = Data::new(RwLock::new(context));

    let subgraph_handle = start_subgraph_runloop(
        subgraph_events_tx,
        subgraph_commands_rx,
        context_wrapped.clone(),
        schema_wrapped.clone(),
        schema_datasource,
    )?;

    let server = HttpServer::new(move || {
        App::new()
            .app_data(schema_wrapped.clone())
            .app_data(context_wrapped.clone())
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allowed_methods(vec!["POST", "GET", "OPTIONS", "DELETE"])
                    .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
                    .allowed_header(header::CONTENT_TYPE)
                    .supports_credentials()
                    .max_age(3600),
            )
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .service(
                web::scope("/gql/v1")
                    .route("/graphql?<request..>", web::get().to(get_graphql))
                    .route("/graphql", web::post().to(post_graphql))
                    .route("/subscriptions", web::get().to(subscriptions)),
            )
            .service(web::resource("/playground").route(web::get().to(playground)))
            .service(web::resource("/graphiql").route(web::get().to(graphiql)))
            .service(dist)
    })
    .workers(5)
    .bind(network_binding)?
    .run();
    let handle = server.handle();
    tokio::spawn(server);
    Ok((handle, subgraph_handle))
}

#[cfg(feature = "explorer")]
fn handle_embedded_file(path: &str) -> HttpResponse {
    use mime_guess::from_path;
    match Asset::get(path) {
        Some(content) => HttpResponse::Ok()
            .content_type(from_path(path).first_or_octet_stream().as_ref())
            .body(content.data.into_owned()),
        None => {
            if let Some(index_content) = Asset::get("index.html") {
                HttpResponse::Ok()
                    .content_type("text/html")
                    .body(index_content.data.into_owned())
            } else {
                HttpResponse::NotFound().body("404 Not Found")
            }
        }
    }
}

#[cfg(not(feature = "explorer"))]
fn handle_embedded_file(_path: &str) -> HttpResponse {
    HttpResponse::NotFound().body("404 Not Found")
}

#[actix_web::get("/{_:.*}")]
async fn dist(path: web::Path<String>) -> impl Responder {
    let path_str = match path.as_str() {
        "" => "index.html",
        other => other,
    };
    handle_embedded_file(path_str)
}

async fn post_graphql(
    req: HttpRequest,
    payload: web::Payload,
    schema: Data<RwLock<Option<GqlDynamicSchema>>>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let schema = schema
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read schema"))?;
    let schema = schema
        .as_ref()
        .ok_or(actix_web::error::ErrorInternalServerError(
            "Missing expected schema",
        ))?;
    graphql_handler(&schema, &context, req, payload).await
}

async fn get_graphql(
    req: HttpRequest,
    payload: web::Payload,
    schema: Data<RwLock<Option<GqlDynamicSchema>>>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let schema = schema
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read schema"))?;
    let schema = schema
        .as_ref()
        .ok_or(actix_web::error::ErrorInternalServerError(
            "Missing expected schema",
        ))?;
    graphql_handler(&schema, &context, req, payload).await
}

async fn subscriptions(
    req: HttpRequest,
    stream: web::Payload,
    schema: Data<GqlDynamicSchema>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let ctx = GqlContext {
        subgraph_name_lookup: context.subgraph_name_lookup.clone(),
        entries_store: context.entries_store.clone(),
        entries_broadcaster: context.entries_broadcaster.clone(),
    };
    let config = ConnectionConfig::new(ctx);
    let config = config.with_keep_alive_interval(Duration::from_secs(15));
    subscriptions::ws_handler(req, stream, schema.into_inner(), config).await
}

async fn playground() -> Result<HttpResponse, Error> {
    playground_handler("/gql/v1/graphql", Some("/gql/v1/subscriptions")).await
}

async fn graphiql() -> Result<HttpResponse, Error> {
    graphiql_handler("/gql/v1/graphql", Some("/gql/v1/subscriptions")).await
}

fn start_subgraph_runloop(
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    gql_context: Data<RwLock<GqlContext>>,
    gql_schema: Data<RwLock<Option<GqlDynamicSchema>>>,
    mut schema_datasource: SchemaDataSource,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let handle = hiro_system_kit::thread_named("Subgraph")
        .spawn(move || {
            let mut observers = vec![];
            loop {
                let mut selector = Select::new();
                let mut handles = vec![];
                selector.recv(&subgraph_commands_rx);
                for rx in observers.iter() {
                    handles.push(selector.recv(rx));
                }
                let oper = selector.select();
                match oper.index() {
                    0 => match oper.recv(&subgraph_commands_rx) {
                        Err(_e) => {
                            // todo
                        }
                        Ok(cmd) => match cmd {
                            SubgraphCommand::CreateSubgraph(uuid, config, sender) => {
                                let err_ctx = "Failed to create new subgraph";
                                let mut gql_schema = gql_schema.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql schema")
                                })?;
                                let subgraph_uuid = uuid;
                                let subgraph_name = config.subgraph_name.clone();
                                let schema = DynamicSchemaMetadata::new(
                                    &subgraph_uuid,
                                    &subgraph_name,
                                    &config.subgraph_description,
                                    &config.fields,
                                );

                                schema_datasource.add_entry(schema);
                                gql_schema.replace(new_dynamic_schema(schema_datasource.clone()));
                                use convert_case::{Case, Casing};

                                let gql_context = gql_context.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql context")
                                })?;
                                let mut entries_store = gql_context.entries_store.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on entries store")
                                })?;
                                let mut lookup = gql_context.subgraph_name_lookup.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on subgraph name lookup")
                                })?;
                                lookup.insert(
                                    subgraph_uuid.clone(),
                                    subgraph_name.to_case(Case::Camel),
                                );
                                entries_store.insert(
                                    subgraph_name.to_case(Case::Camel),
                                    (subgraph_uuid.clone(), vec![]),
                                );
                                let _ = sender.send("http://127.0.0.1:8900/playground".into());
                            }
                            SubgraphCommand::ObserveSubgraph(subgraph_observer_rx) => {
                                observers.push(subgraph_observer_rx);
                            }
                            SubgraphCommand::Shutdown => {
                                let _ = subgraph_events_tx.send(SubgraphEvent::Shutdown);
                            }
                        },
                    },
                    i => match oper.recv(&observers[i - 1]) {
                        Ok(cmd) => match cmd {
                            SchemaDataSourcingEvent::ApplyEntry(
                                uuid,
                                values, /* , request, slot*/
                            ) => {
                                let err_ctx = "Failed to apply new database entry to subgraph";
                                let gql_context = gql_context.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql context")
                                })?;
                                let uuid_lookup = gql_context.subgraph_name_lookup.read().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire read lock on subgraph name lookup")
                                })?;
                                let subgraph_name = uuid_lookup.get(&uuid).ok_or_else(|| {
                                    format!("{err_ctx}: Subgraph name not found for uuid: {}", uuid)
                                })?;
                                let mut entries_store = gql_context.entries_store.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on entries store")
                                })?;
                                let (_, entries) = entries_store.get_mut(subgraph_name).ok_or_else(|| {
                                    format!("{err_ctx}: Entries not found for subgraph: {}", subgraph_name)
                                })?;
                                let values: HashMap<String, Value> = serde_json::from_slice(values.as_slice()).map_err(|e| {
                                    format!("{err_ctx}: Failed to deserialize new database entry for subgraph {}: {}", subgraph_name, e)
                                })?;
                                entries.push(GqlSubgraphDataEntry(SubgraphDataEntry::new(values)));
                            }
                            SchemaDataSourcingEvent::Rountrip(_uuid) => {}
                        },
                        Err(_e) => {}
                    },
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        })
        .map_err(|e| format!("Subgraph processing thread failed: {}", e))?;

    Ok(handle)
}
