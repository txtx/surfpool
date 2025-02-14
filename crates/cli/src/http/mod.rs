use crate::cli::Context;
use actix_cors::Cors;
use actix_web::dev::ServerHandle;
use actix_web::http::header::{self};
use actix_web::web::{self, Data};
use actix_web::HttpRequest;
use actix_web::{middleware, App, HttpResponse, HttpServer};
use actix_web::{Error, Responder};
use crossbeam::channel::{Receiver, Sender};
use ipc_channel::ipc::IpcReceiver;
use juniper_actix::{graphiql_handler, graphql_handler, playground_handler, subscriptions};
use juniper_graphql_ws::ConnectionConfig;
use std::error::Error as StdError;
use std::sync::RwLock;
use std::time::Duration;
use surfpool_core::types::{Collection, SubgraphCommand, SubgraphEvent, SurfpoolConfig};
use surfpool_gql::types::collection::CollectionData;
use surfpool_gql::{new_graphql_schema, Context as GqlContext, GqlSchema};
use txtx_core::kit::uuid::Uuid;

#[cfg(feature = "explorer")]
use rust_embed::RustEmbed;

#[cfg(feature = "explorer")]
#[derive(RustEmbed)]
#[folder = "../../../explorer/.next/server/app"]
pub struct Asset;

pub async fn start_server(
    network_binding: String,
    config: SurfpoolConfig,
    subgraph_events_tx: Sender<SubgraphEvent>,
    subgraph_commands_rx: Receiver<SubgraphCommand>,
    _ctx: &Context,
) -> Result<ServerHandle, Box<dyn StdError>> {
    let gql_schema = Data::new(new_graphql_schema());
    let gql_context: Data<RwLock<GqlContext>> = Data::new(RwLock::new(GqlContext::new()));

    let gql_context_copy = gql_context.clone();
    let _handle = hiro_system_kit::thread_named("subgraph")
        .spawn(move || {
            loop {
                match subgraph_commands_rx.recv() {
                    Err(_e) => {
                        // todo
                    }
                    Ok(cmd) => match cmd {
                        SubgraphCommand::CreateEndpoint(config, sender) => {
                            println!("Here: {:?}", config);
                            let gql_context = gql_context_copy.write().unwrap();
                            let mut collections = gql_context.collections_store.write().unwrap();
                            let collection_uuid = Uuid::new_v4();

                            collections.insert(
                                collection_uuid.clone(),
                                CollectionData {
                                    collection: Collection {
                                        uuid: collection_uuid,
                                        name: config.subgraph_name.clone(),
                                        entries: vec![],
                                    },
                                },
                            );
                            println!("{:?}", collections);
                            let _ = sender.send("http://127.0.0.1:8900/graphql".into());
                        }
                        SubgraphCommand::Shutdown => {
                            let _ = subgraph_events_tx.send(SubgraphEvent::Shutdown);
                        }
                    },
                }
            }
            Ok::<(), String>(())
        })
        .map_err(|e| format!("{}", e))?;

    let server = HttpServer::new(move || {
        App::new()
            .app_data(gql_schema.clone())
            .app_data(gql_context.clone())
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
    Ok(handle)
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
    schema: Data<GqlSchema>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    println!("POST /graphql");
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    graphql_handler(&schema, &context, req, payload).await
}

async fn get_graphql(
    req: HttpRequest,
    payload: web::Payload,
    schema: Data<GqlSchema>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    println!("GET /graphql");
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    graphql_handler(&schema, &context, req, payload).await
}

async fn subscriptions(
    req: HttpRequest,
    stream: web::Payload,
    schema: Data<GqlSchema>,
    context: Data<RwLock<GqlContext>>,
) -> Result<HttpResponse, Error> {
    let context = context
        .read()
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to read context"))?;
    let ctx = GqlContext {
        collections_store: context.collections_store.clone(),
        entries_broadcaster: context.entries_broadcaster.clone(),
    };
    let config = ConnectionConfig::new(ctx);
    let config = config.with_keep_alive_interval(Duration::from_secs(15));
    subscriptions::ws_handler(req, stream, schema.into_inner(), config).await
}

async fn playground() -> Result<HttpResponse, Error> {
    playground_handler("/graphql", Some("/subscriptions")).await
}

async fn graphiql() -> Result<HttpResponse, Error> {
    graphiql_handler("/graphql", Some("/subscriptions")).await
}
