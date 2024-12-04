use super::endpoints;
use super::mavlink_vehicle::MAVLinkVehicleArcMutex;

use paperclip::actix::{ web, web::Scope, OpenApiExt };

use actix_cors::Cors;
use actix_web::{
    error::{ ErrorBadRequest, JsonPayloadError },
    rt::System,
    App,
    HttpRequest,
    HttpServer,
};

use crate::cli;
use log::*;

fn json_error_handler(error: JsonPayloadError, _: &HttpRequest) -> actix_web::Error {
    warn!("Problem with json: {}", error.to_string());
    match error {
        JsonPayloadError::Overflow => JsonPayloadError::Overflow.into(),
        _ => ErrorBadRequest(error.to_string()),
    }
}

fn add_v1_paths(scope: Scope) -> Scope {
    scope
        .route("/helper/mavlink", web::get().to(endpoints::helper_mavlink))
        .route("/mavlink", web::get().to(endpoints::mavlink))
        .route("/mavlink", web::post().to(endpoints::mavlink_post))
        .route("/mission_clear", web::post().to(endpoints::mission_clear))
        .route("/mission_post", web::post().to(endpoints::mission_post))
        .route("/mission_get", web::get().to(endpoints::mission_get))
        .route("/flyto", web::post().to(endpoints::fly_to))
        .route("/set_fence", web::post().to(endpoints::set_fence))
        .route(r"/mavlink/{path:.*}", web::get().to(endpoints::mavlink))
        .service(web::resource("/ws/mavlink").route(web::get().to(endpoints::websocket)))
        // New group management routes
        .route("/groups", web::post().to(endpoints::create_group))
        .route("/groups", web::get().to(endpoints::list_groups))
        .route("/groups/{group_id}", web::delete().to(endpoints::delete_group))
        .route("/groups/{group_id}", web::put().to(endpoints::update_group))
        .route("/groups/{group_id}/vehicles", web::get().to(endpoints::list_vehicles))
        .route("/groups/{group_id}/vehicles", web::post().to(endpoints::add_vehicle))
        .route(
            "/groups/{group_id}/vehicles/{vehicle_id}",
            web::delete().to(endpoints::remove_vehicle)
        )
        // Set/get leader of group
        .route("/groups/{group_id}/leader", web::put().to(endpoints::set_leader))
        .route("/groups/{group_id}/leader", web::get().to(endpoints::get_leader))
}

// Start REST API server with the desired address
pub fn run(server_address: &str, mavlink_vehicle: &MAVLinkVehicleArcMutex) {
    let server_address = server_address.to_string();
    let mavlink_vehicle = mavlink_vehicle.clone();
    println!("Server running: http://{server_address}");

    // Start HTTP server thread
    let _ = System::new("http-server");
    HttpServer::new(move || {
        let v1 = add_v1_paths(web::scope("/v1"));
        let default = match cli::default_api_version() {
            1 => add_v1_paths(web::scope("")),
            _ => unreachable!("CLI should only allow supported values."),
        };
        App::new()
            .wrap(Cors::permissive())
            // Record services and routes for paperclip OpenAPI plugin for Actix.
            .wrap_api()
            //TODO Add middle man to print all http events
            .data(web::JsonConfig::default().error_handler(json_error_handler))
            .data(mavlink_vehicle.clone())
            //TODO: Add cors
            .route("/", web::get().to(endpoints::root))
            .with_json_spec_at("/docs.json")
            .with_swagger_ui_at("/docs")
            .route(r"/{filename:.*(\.html|\.js|\.css)}", web::get().to(endpoints::root))
            .route("/info", web::get().to(endpoints::info))
            // Be sure to have default as the latest endpoint, otherwise it does not work
            .service(v1)
            .service(default)
            .build()
    })
        .bind(server_address)
        .unwrap()
        .run();
}
