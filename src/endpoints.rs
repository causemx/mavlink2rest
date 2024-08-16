use std::path::Path;
use actix_web::{
    web::{self, Json},
    HttpRequest, HttpResponse,
};
use actix_web_actors::ws;
use include_dir::{include_dir, Dir};
use paperclip::actix::{api_v2_operation, Apiv2Schema};
use serde::{Deserialize, Serialize};

use super::data;
use super::mavlink_vehicle::MAVLinkVehicleArcMutex;
use super::websocket_manager::WebsocketActor;

use log::*;
use mavlink::{MavHeader, Message};

static HTML_DIST: Dir<'_> = include_dir!("src/html");

#[derive(Apiv2Schema, Serialize, Debug, Default)]
pub struct InfoContent {
    /// Name of the program
    name: String,
    /// Version/tag
    version: String,
    /// Git SHA
    sha: String,
    build_date: String,
    /// Authors name
    authors: String,
}

#[derive(Apiv2Schema, Serialize, Debug, Default)]
pub struct Info {
    /// Version of the REST API
    version: u32,
    /// Service information
    service: InfoContent,
}

#[derive(Apiv2Schema, Deserialize)]
pub struct WebsocketQuery {
    /// Regex filter to selected the desired MAVLink messages by name
    filter: Option<String>,
}

#[derive(Apiv2Schema, Deserialize)]
pub struct MAVLinkHelperQuery {
    /// MAVLink message name, possible options are here: https://docs.rs/mavlink/0.10.0/mavlink/#modules
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Waypoint {
    target_system: u8,
    target_component: u8,
    seq: u16,
    frame: u8,
    command: u16,
    current: u8,
    autocontinue: u8,
    param1: f32,
    param2: f32,
    param3: f32,
    param4: f32,
    x: f64,
    y: f64,
    z: f32,
    mission_type: u8,
}

fn load_html_file(filename: &str) -> Option<String> {
    if let Some(file) = HTML_DIST.get_file(filename) {
        return Some(file.contents_utf8().unwrap().to_string());
    }

    None
}

pub fn root(req: HttpRequest) -> HttpResponse {
    let mut filename = req.match_info().query("filename");
    if filename.is_empty() {
        filename = "index.html";
    }

    if let Some(content) = load_html_file(filename) {
        let extension = Path::new(&filename)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("");
        let mime = actix_files::file_extension_to_mime(extension).to_string();

        return HttpResponse::Ok().content_type(mime).body(content);
    };

    return HttpResponse::NotFound()
        .content_type("text/plain")
        .body("File does not exist");
}

#[api_v2_operation]
/// Provides information about the API and this program
pub async fn info() -> Json<Info> {
    let info = Info {
        version: 0,
        service: InfoContent {
            name: env!("CARGO_PKG_NAME").into(),
            version: env!("VERGEN_GIT_SEMVER").into(),
            sha: env!("VERGEN_GIT_SHA").into(),
            build_date: env!("VERGEN_BUILD_TIMESTAMP").into(),
            authors: env!("CARGO_PKG_AUTHORS").into(),
        },
    };

    Json(info)
}

#[api_v2_operation]
/// Provide information related to GPS(coordinate),
/// include: lat: latitude, lon: longitude
pub async fn get_gps(_req: HttpRequest) -> actix_web::Result<HttpResponse> {
   let path = "vehicles/1/components/1/messages/GPS_RAW_INT/message";
   let message = data::messages().pointer(&path); 
   ok_response(message).await
}

#[api_v2_operation]
/// Provide information related to SPEED,
/// include: airspeed, groundspeed
pub async fn get_speed(_req: HttpRequest) -> actix_web::Result<HttpResponse> {
    let path = "vehicles/1/components/1/messages/VFR_HUD/message";
    let message = data::messages().pointer(&path); 
    ok_response(message).await 
}

#[api_v2_operation]
/// Provided information related to BATTERY,
/// include: voltage_battery, current_battery, battery_remain
pub async fn get_voltage(_req: HttpRequest) -> actix_web::Result<HttpResponse> {
    let path = "vehicles/1/components/1/messages/SYS_STATUS/message";
    let message = data::messages().pointer(&path); 
    ok_response(message).await 
}

#[api_v2_operation]
/// Provided information related to ATTITUDE,
/// include: roll, pitch, yaw 
pub async fn get_altitude(_req: HttpRequest) -> actix_web::Result<HttpResponse> {
    let path = "vehicles/1/components/1/messages/ATTITUDE/message";
    let message = data::messages().pointer(&path); 
    ok_response(message).await 
}

#[api_v2_operation]
/// Provides an object containing all MAVLink messages received by the service
pub async fn mavlink(req: HttpRequest) -> actix_web::Result<HttpResponse> {
    let path = req.match_info().query("path");
    let message = data::messages().pointer(path);
    ok_response(message).await
}

pub fn parse_query<T: serde::ser::Serialize>(message: &T) -> String {
    let error_message =
        "Not possible to parse mavlink message, please report this issue!".to_string();
    serde_json::to_string_pretty(&message).unwrap_or(error_message)
}

#[api_v2_operation]
/// Returns a MAVLink message matching the given message name
pub async fn helper_mavlink(
    _req: HttpRequest,
    query: web::Query<MAVLinkHelperQuery>,
) -> actix_web::Result<HttpResponse> {
    let message_name = query.into_inner().name;

    let result = match mavlink::ardupilotmega::MavMessage::message_id_from_name(&message_name) {
        Ok(id) => mavlink::Message::default_message_from_id(id),
        Err(error) => Err(error),
    };

    match result {
        Ok(result) => {
            let msg = match result {
                mavlink::ardupilotmega::MavMessage::common(msg) => {
                    parse_query(&data::MAVLinkMessage {
                        header: mavlink::MavHeader::default(),
                        message: msg,
                    })
                }
                msg => parse_query(&data::MAVLinkMessage {
                    header: mavlink::MavHeader::default(),
                    message: msg,
                }),
            };

            ok_response(msg).await
        }
        Err(content) => not_found_response(parse_query(&content)).await,
    }
}

#[api_v2_operation]
#[allow(clippy::await_holding_lock)]
pub async fn mission_post(
    data: web::Data<MAVLinkVehicleArcMutex>,
    _req: HttpRequest,
    bytes: web::Bytes,
) -> actix_web::Result<HttpResponse> {
    let json_string = String::from_utf8(bytes.to_vec()).expect("Failed to parse by UTF8");
    debug!("received: {json_string}");

    let waypoints: Vec<Waypoint> = serde_json::from_str(&json_string)?;
    let mission_count = waypoints.len();
    let mission_count_msg =
        mavlink::ardupilotmega::MavMessage::common(mavlink::common::MavMessage::MISSION_COUNT(mavlink::common::MISSION_COUNT_DATA {
            count: mission_count as u16,
            target_component: 1,
            target_system: 1,
            mission_type: mavlink::common::MavMissionType::MAV_MISSION_TYPE_MISSION,
        }));
    match data
        .lock()
        .unwrap()
        .send(&MavHeader::default(), &mission_count_msg)
    {
        Ok(_result) => {
            // data::update((content.header, content_ardupilotmega));
            return HttpResponse::Ok().await;
        }
        Err(err) => {
            return not_found_response(format!("Failed to send message: {err:?}")).await;
        }
    }
}

#[api_v2_operation]
#[allow(clippy::await_holding_lock)]
/// Send a MAVLink message for the desired vehicle
pub async fn mavlink_post(
    data: web::Data<MAVLinkVehicleArcMutex>,
    _req: HttpRequest,
    bytes: web::Bytes,
) -> actix_web::Result<HttpResponse> {
    let json_string = match String::from_utf8(bytes.to_vec()) {
        Ok(content) => content,
        Err(err) => {
            return not_found_response(format!("Failed to parse input as UTF-8 string: {err:?}"))
                .await;
        }
    };

    debug!("MAVLink post received: {json_string}");

    if let Ok(content) =
        json5::from_str::<data::MAVLinkMessage<mavlink::ardupilotmega::MavMessage>>(&json_string)
    {
        match data.lock().unwrap().send(&content.header, &content.message) {
            Ok(_result) => {
                data::update((content.header, content.message));
                return HttpResponse::Ok().await;
            }
            Err(err) => {
                return not_found_response(format!("Failed to send message: {err:?}")).await
            }
        }
    }

    if let Ok(content) =
        json5::from_str::<data::MAVLinkMessage<mavlink::common::MavMessage>>(&json_string)
    {
        let content_ardupilotmega = mavlink::ardupilotmega::MavMessage::common(content.message);
        match data
            .lock()
            .unwrap()
            .send(&content.header, &content_ardupilotmega)
        {
            Ok(_result) => {
                data::update((content.header, content_ardupilotmega));
                return HttpResponse::Ok().await;
            }
            Err(err) => {
                return not_found_response(format!("Failed to send message: {err:?}")).await;
            }
        }
    }

    not_found_response(String::from(
        "Failed to parse message, not a valid MAVLinkMessage.",
    ))
    .await
}

#[api_v2_operation]
/// Drone Assembly
pub async fn assembly() -> actix_web::Result<HttpResponse> {
    ok_response("assemb".to_string()).await
}

#[api_v2_operation]
/// Websocket used to receive and send MAVLink messages asynchronously
pub async fn websocket(
    req: HttpRequest,
    query: web::Query<WebsocketQuery>,
    stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    let filter = match query.into_inner().filter {
        Some(filter) => filter,
        _ => ".*".to_owned(),
    };

    debug!("New websocket with filter {:#?}", &filter);

    ws::start(WebsocketActor::new(filter), &req, stream)
}

async fn not_found_response(message: String) -> actix_web::Result<HttpResponse> {
    HttpResponse::NotFound()
        .content_type("application/json")
        .body(message)
        .await
}

async fn ok_response(message: String) -> actix_web::Result<HttpResponse> {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(message)
        .await
}
