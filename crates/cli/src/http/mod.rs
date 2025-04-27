#![allow(unused_imports, unused_variables)]

use crate::cli::Context;
use actix_cors::Cors;
use actix_web::dev::ServerHandle;
use actix_web::http::header::{self};
use actix_web::web::{self, Data};
use actix_web::HttpRequest;
use actix_web::{middleware, App, HttpResponse, HttpServer};
use actix_web::{Error, Responder};
use crossbeam::channel::{Receiver, Select, Sender};
use juniper_actix::{graphiql_handler, graphql_handler, subscriptions};
use juniper_graphql_ws::ConnectionConfig;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::sync::RwLock;
use std::thread::JoinHandle;
use std::time::Duration;
use surfpool_gql::query::{DataloaderContext, MemoryStore, SchemaDataSource};
use surfpool_gql::types::schema::DynamicSchemaSpec;
use surfpool_gql::types::SubgraphSpec;
use surfpool_gql::{new_dynamic_schema, DynamicSchema};
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
    let context: DataloaderContext = Box::new(MemoryStore::new());
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
            .service(web::resource("/gql/console").route(web::get().to(graphiql)))
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
    schema: Data<RwLock<Option<DynamicSchema>>>,
    context: Data<RwLock<DataloaderContext>>,
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
    graphql_handler(schema, &context, req, payload).await
}

async fn get_graphql(
    req: HttpRequest,
    payload: web::Payload,
    schema: Data<RwLock<Option<DynamicSchema>>>,
    context: Data<RwLock<DataloaderContext>>,
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
    graphql_handler(schema, &context, req, payload).await
}

async fn subscriptions(
    req: HttpRequest,
    stream: web::Payload,
    schema: Data<DynamicSchema>,
    context: Data<RwLock<DataloaderContext>>,
) -> Result<HttpResponse, Error> {
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let config = ConnectionConfig::new(context);
    let config = config.with_keep_alive_interval(Duration::from_secs(15));
    unimplemented!()
    // subscriptions::ws_handler(req, stream, schema.into_inner(), config).await
}

async fn graphiql() -> Result<HttpResponse, Error> {
    graphiql_handler("/gql/v1/graphql", Some("/gql/v1/subscriptions")).await
}

fn start_subgraph_runloop(
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    gql_context: Data<RwLock<DataloaderContext>>,
    gql_schema: Data<RwLock<Option<DynamicSchema>>>,
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
                            SubgraphCommand::CreateSubgraph(uuid, request, sender) => {
                                let err_ctx = "Failed to create new subgraph";
                                let mut gql_schema = gql_schema.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql schema")
                                })?;

                                let subgraph_uuid = uuid;
                                schema_datasource.add_entry(DynamicSchemaSpec::from_request(&uuid, &request));
                                gql_schema.replace(new_dynamic_schema(schema_datasource.clone()));

                                let gql_context = gql_context.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql context")
                                })?;
                                gql_context.register_subgraph(&request.subgraph_name, subgraph_uuid)?;
                                let _ = sender.send("http://127.0.0.1:8900/gql/console".into());
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
                                values,
                                slot,
                                tx_hash
                            ) => {
                                let err_ctx = "Failed to apply new database entry to subgraph";
                                let gql_context = gql_context.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql context")
                                })?;
                                let subgraph_name = gql_context.get_subgraph_name(&uuid).ok_or_else(|| {
                                    format!("{err_ctx}: Subgraph name not found for uuid: {}", uuid)
                                })?;
                                let entries: Vec<HashMap<String, Value>> = serde_json::from_slice(values.as_slice()).map_err(|e| {
                                    format!("{err_ctx}: Failed to deserialize new database entry for subgraph {}: {}", subgraph_name, e)
                                })?;
                                for entry in entries.into_iter() {
                                    gql_context.insert_entry_to_subgraph(&subgraph_name, SubgraphSpec(SubgraphDataEntry::new(entry, slot, tx_hash.clone())))?;
                                }
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
