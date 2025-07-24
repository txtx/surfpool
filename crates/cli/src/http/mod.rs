#![allow(unused_imports, unused_variables)]

use std::{
    collections::HashMap, error::Error as StdError, sync::RwLock, thread::JoinHandle,
    time::Duration,
};

use actix_cors::Cors;
use actix_web::{
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder,
    dev::ServerHandle,
    http::header::{self},
    middleware,
    web::{self, Data, route},
};
use convert_case::{Case, Casing};
use crossbeam::channel::{Receiver, Select, Sender};
use juniper_actix::{graphiql_handler, graphql_handler, subscriptions};
use juniper_graphql_ws::ConnectionConfig;
#[cfg(feature = "explorer")]
use rust_embed::RustEmbed;
use surfpool_gql::{
    DynamicSchema,
    db::schema::collections,
    new_dynamic_schema,
    query::{CollectionMetadataMap, Dataloader, DataloaderContext, SqlStore},
    types::{CollectionEntry, CollectionEntryData, collections::CollectionMetadata},
};
use surfpool_studio_ui::serve_studio_static_files;
use surfpool_types::{
    DataIndexingCommand, SanitizedConfig, SubgraphCommand, SubgraphEvent, SurfpoolConfig,
};
use txtx_core::kit::types::types::Value;

use crate::cli::Context;

#[cfg(feature = "explorer")]
#[derive(RustEmbed)]
#[folder = "../../../explorer/.next/server/app"]
pub struct Asset;

pub async fn start_subgraph_and_explorer_server(
    network_binding: String,
    subgraph_database_path: &str,
    config: SanitizedConfig,
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    ctx: &Context,
) -> Result<(ServerHandle, JoinHandle<Result<(), String>>), Box<dyn StdError>> {
    let context = DataloaderContext {
        pool: SqlStore::new(subgraph_database_path).pool,
    };
    let schema_datasource = CollectionMetadataMap::new();
    let schema = RwLock::new(Some(new_dynamic_schema(schema_datasource.clone())));
    let schema_wrapped = Data::new(schema);
    let context_wrapped = Data::new(RwLock::new(context));
    let config_wrapped = Data::new(RwLock::new(config.clone()));

    let subgraph_handle = start_subgraph_runloop(
        subgraph_events_tx,
        subgraph_commands_rx,
        context_wrapped.clone(),
        schema_wrapped.clone(),
        schema_datasource,
        config,
        ctx,
    )?;

    let server = HttpServer::new(move || {
        App::new()
            .app_data(schema_wrapped.clone())
            .app_data(context_wrapped.clone())
            .app_data(config_wrapped.clone())
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
            .service(get_config)
            .service(
                web::scope("/gql")
                    .route("/v1/graphql?<request..>", web::get().to(get_graphql))
                    .route("/v1/graphql", web::post().to(post_graphql))
                    .route("/v1/subscriptions", web::get().to(subscriptions))
                    .route("/console", web::get().to(graphiql)),
            )
            .service(serve_studio_static_files)
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

#[actix_web::get("/config")]
async fn get_config(
    req: HttpRequest,
    payload: web::Payload,
    config: Data<RwLock<SanitizedConfig>>,
) -> Result<HttpResponse, Error> {
    let config = config
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let api_config = serde_json::json!(*config);
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(api_config.to_string()))
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
    graphiql_handler("/gql/v1/graphql", None).await
}

fn start_subgraph_runloop(
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    gql_context: Data<RwLock<DataloaderContext>>,
    gql_schema: Data<RwLock<Option<DynamicSchema>>>,
    mut collections_map: CollectionMetadataMap,
    config: SanitizedConfig,
    ctx: &Context,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let ctx = ctx.clone();
    let handle = hiro_system_kit::thread_named("Subgraph")
        .spawn(move || {
            let mut observers = vec![];
            let mut cached_metadata = HashMap::new();
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
                            std::process::exit(1);
                        }
                        Ok(cmd) => match cmd {
                            SubgraphCommand::CreateCollection(uuid, request, sender) => {
                                let err_ctx = "Failed to create new subgraph";
                                let mut gql_schema = gql_schema.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql schema")
                                })?;

                                let metadata = CollectionMetadata::from_request(&uuid, &request);

                                cached_metadata.insert(uuid.clone(), metadata.clone());

                                let gql_context = gql_context.write().map_err(|_| {
                                    format!(
                                        "{err_ctx}: Failed to acquire write lock on gql context"
                                    )
                                })?;
                                if let Err(e) = gql_context.pool.register_collection(&metadata) {
                                    error!(ctx.expect_logger(), "{}", e);
                                }
                                collections_map.add_collection(metadata);
                                gql_schema.replace(new_dynamic_schema(collections_map.clone()));

                                let console_url = format!("{}/subgraphs", config.studio_url.clone());
                                let _ = sender.send(console_url);
                            }
                            SubgraphCommand::ObserveCollection(subgraph_observer_rx) => {
                                observers.push(subgraph_observer_rx);
                            }
                            SubgraphCommand::Shutdown => {
                                let _ = subgraph_events_tx.send(SubgraphEvent::Shutdown);
                            }
                        },
                    },
                    i => match oper.recv(&observers[i - 1]) {
                        Ok(cmd) => match cmd {
                            DataIndexingCommand::ProcessCollectionEntriesPack(
                                uuid,
                                entry_bytes,
                            ) => {
                                let err_ctx = "Failed to apply new database entry to subgraph";
                                let gql_context = match gql_context.write() {
                                    Ok(ctx) => ctx,
                                    Err(_) => {
                                        let _ = subgraph_events_tx.send(SubgraphEvent::error(format!(
                                        "{err_ctx}: Failed to acquire write lock on gql context"
                                    )));
                                        continue;
                                    }
                                };

                                let entries = match CollectionEntryData::from_entries_bytes(&uuid, entry_bytes) {
                                    Ok(entries) => entries,
                                    Err(e) => {
                                        let _ = subgraph_events_tx.send(SubgraphEvent::error(format!(
                                            "{err_ctx}: {e}"
                                        )));
                                        continue;
                                    }
                                };

                                let metadata = cached_metadata.get(&uuid).unwrap();

                                if let Err(e) = gql_context
                                    .pool
                                    .insert_entries_into_collection(entries, &metadata)
                                {
                                    let _ = subgraph_events_tx.send(SubgraphEvent::error(format!(
                                        "{err_ctx}: {e}"
                                    )));
                                    continue;
                                }
                            }
                            DataIndexingCommand::ProcessCollection(_uuid) => {}
                        },
                        Err(_e) => {
                            std::process::exit(1);
                        }
                    },
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        })
        .map_err(|e| format!("Subgraph processing thread failed: {}", e))?;

    Ok(handle)
}
