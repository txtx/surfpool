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
use log::{debug, error, info, trace, warn};
#[cfg(feature = "explorer")]
use rust_embed::RustEmbed;
use surfpool_core::scenarios::TemplateRegistry;
use surfpool_gql::{
    DynamicSchema,
    db::schema::collections,
    new_dynamic_schema,
    query::{CollectionsMetadataLookup, Dataloader, DataloaderContext, SqlStore},
    types::{CollectionEntry, CollectionEntryData, collections::CollectionMetadata, sql},
};
use surfpool_studio_ui::serve_studio_static_files;
use surfpool_types::{
    DataIndexingCommand, OverrideTemplate, SanitizedConfig, SubgraphCommand, SubgraphEvent,
    SurfpoolConfig,
};
use txtx_core::kit::types::types::Value;
use txtx_gql::kit::uuid::Uuid;

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
    enable_studio: bool,
) -> Result<(ServerHandle, JoinHandle<Result<(), String>>), Box<dyn StdError>> {
    let sql_store = SqlStore::new(subgraph_database_path);
    sql_store.init_subgraph_tables()?;

    let context = DataloaderContext {
        pool: sql_store.pool,
    };
    let collections_metadata_lookup = CollectionsMetadataLookup::new();
    let schema = RwLock::new(Some(new_dynamic_schema(
        collections_metadata_lookup.clone(),
    )));
    let schema_wrapped = Data::new(schema);
    let context_wrapped = Data::new(RwLock::new(context));
    let config_wrapped = Data::new(RwLock::new(config.clone()));
    let collections_metadata_lookup_wrapped = Data::new(RwLock::new(collections_metadata_lookup));

    // Initialize template registry and load templates
    let template_registry_wrapped = Data::new(RwLock::new(TemplateRegistry::new()));

    let subgraph_handle = start_subgraph_runloop(
        subgraph_events_tx,
        subgraph_commands_rx,
        context_wrapped.clone(),
        schema_wrapped.clone(),
        collections_metadata_lookup_wrapped.clone(),
        config,
        ctx,
    )?;

    let server = HttpServer::new(move || {
        let mut app = App::new()
            .app_data(schema_wrapped.clone())
            .app_data(context_wrapped.clone())
            .app_data(config_wrapped.clone())
            .app_data(collections_metadata_lookup_wrapped.clone())
            .app_data(template_registry_wrapped.clone())
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
            .service(get_indexers)
            .service(get_scenario_templates)
            .service(
                web::scope("/workspace")
                    .route("/v1/indexers", web::post().to(post_graphql))
                    .route("/v1/graphql?<request..>", web::get().to(get_graphql))
                    .route("/v1/graphql", web::post().to(post_graphql))
                    .route("/v1/subscriptions", web::get().to(subscriptions)),
            );

        if enable_studio {
            app = app.service(serve_studio_static_files);
        }

        app
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

#[actix_web::get("/workspace/v1/indexers")]
async fn get_indexers(
    collections_metadata_lookup: Data<RwLock<CollectionsMetadataLookup>>,
) -> Result<HttpResponse, Error> {
    let lookup = collections_metadata_lookup.read().map_err(|_| {
        actix_web::error::ErrorInternalServerError("Failed to read collections metadata")
    })?;

    let collections: Vec<&CollectionMetadata> = lookup.entries.values().collect();
    let response = serde_json::to_string(&collections).map_err(|_| {
        actix_web::error::ErrorInternalServerError("Failed to serialize collections")
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(response))
}

#[actix_web::get("/v1/scenarios/templates")]
async fn get_scenario_templates(
    template_registry: Data<RwLock<TemplateRegistry>>,
) -> Result<HttpResponse, Error> {
    let registry = template_registry.read().map_err(|_| {
        actix_web::error::ErrorInternalServerError("Failed to read template registry")
    })?;

    let templates: Vec<&OverrideTemplate> = registry.all();
    let response = serde_json::to_string(&templates)
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to serialize templates"))?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(response))
}

#[allow(dead_code)]
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

#[allow(clippy::await_holding_lock)]
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

#[allow(clippy::await_holding_lock)]
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

fn start_subgraph_runloop(
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    gql_context: Data<RwLock<DataloaderContext>>,
    gql_schema: Data<RwLock<Option<DynamicSchema>>>,
    collections_metadata_lookup: Data<RwLock<CollectionsMetadataLookup>>,
    config: SanitizedConfig,
    ctx: &Context,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let ctx = ctx.clone();
    let worker_id = Uuid::default();
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

                                let metadata = CollectionMetadata::from_request(&uuid, &request, "surfpool");
                                cached_metadata.insert(uuid, metadata.clone());

                                let gql_context = gql_context.write().map_err(|_| {
                                    format!(
                                        "{err_ctx}: Failed to acquire write lock on gql context"
                                    )
                                })?;
                                if let Err(e) = gql_context.pool.register_collection(&metadata, &request, &worker_id) {
                                    error!("{}", e);
                                }

                                let mut lookup = collections_metadata_lookup.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on collections metadata lookup")
                                })?;
                                lookup.add_collection(metadata);
                                gql_schema.replace(new_dynamic_schema(lookup.clone()));

                                let console_url = format!("{}/accounts", config.studio_url.clone());
                                let _ = sender.send(console_url);
                            }
                            SubgraphCommand::ObserveCollection(subgraph_observer_rx) => {
                                observers.push(subgraph_observer_rx);
                            }
                            SubgraphCommand::DestroyCollection(uuid) => {
                                let err_ctx = "Failed to destroy subgraph collection";

                                // Remove from cached metadata
                                cached_metadata.remove(&uuid);

                                // Unregister from database pool
                                let gql_context = gql_context.write().map_err(|_| {
                                    format!(
                                        "{err_ctx}: Failed to acquire write lock on gql context"
                                    )
                                })?;
                                if let Err(e) = gql_context.pool.unregister_collection(&uuid) {
                                    error!("{}: {}", err_ctx, e);
                                }

                                // Remove from metadata lookup and update schema
                                let mut gql_schema = gql_schema.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on gql schema")
                                })?;
                                let mut lookup = collections_metadata_lookup.write().map_err(|_| {
                                    format!("{err_ctx}: Failed to acquire write lock on collections metadata lookup")
                                })?;
                                lookup.remove_collection(&uuid);
                                gql_schema.replace(new_dynamic_schema(lookup.clone()));
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

                                // Check if metadata still exists (collection might have been destroyed)
                                let Some(metadata) = cached_metadata.get(&uuid) else {
                                    // Collection was destroyed, skip processing this entry
                                    continue;
                                };

                                if let Err(e) = gql_context
                                    .pool
                                    .insert_entries_into_collection(entries, metadata)
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
                            // Observer channel closed (plugin was likely unloaded)
                            // Remove observer in that case
                            observers.remove(i - 1);
                            continue;
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
