use actix_web::{ web::{ self, Json }, HttpRequest, HttpResponse };
use actix_web_actors::ws;
use include_dir::{ include_dir, Dir };
use jsonwebtoken::{ decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation };
use paperclip::actix::{ api_v2_operation, Apiv2Schema };
use regex::Regex;
use serde::{ Deserialize, Serialize };
use std::path::Path;

use super::data;
use super::mavlink_vehicle::MAVLinkVehicleArcMutex;
use super::websocket_manager::WebsocketActor;

use log::*;
use mavlink::{ common::PositionTargetTypemask, Message };
use std::time::Duration;

#[allow(dead_code)]
const TIMEOUT: Duration = Duration::from_secs(5);

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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    x: i32,
    y: i32,
    z: f32,
    mission_type: u8,
}

#[derive(Deserialize)]
struct TargetLocation {
    lat: f64,
    lon: f64,
    alt: f32,
}
#[allow(dead_code)]
#[derive(Apiv2Schema, Deserialize)]
pub struct JWTInfo {
    connect_string: String,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
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
    }

    return HttpResponse::NotFound().content_type("text/plain").body("File does not exist");
}

#[allow(dead_code)]
fn create_jwt(user_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let pattern = Regex::new(r"^udpin://\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}:\d+$");

    if !pattern?.is_match(user_id) {
        return Err("Invalid format. Expected: udpin://ip:port_number".into());
    }

    let expiration = chrono::Utc
        ::now()
        .checked_add_signed(chrono::Duration::hours(1))
        .expect("valid timestamp")
        .timestamp();

    let claims = Claims {
        sub: user_id.to_owned(),
        exp: expiration as usize,
    };

    let header = Header::new(Algorithm::HS256);
    encode(&header, &claims, &EncodingKey::from_secret("secret".as_ref())).map_err(|_|
        "Failed to connect".into()
    )
}

#[api_v2_operation]
pub async fn connect(info: web::Json<JWTInfo>) -> actix_web::Result<HttpResponse> {
    let token = match create_jwt(&info.connect_string) {
        Ok(t) => t,
        Err(e) => {
            return Ok(
                HttpResponse::BadRequest().json(
                    serde_json::json!({
                "error": format!("Failed to connect vehilce: {}", e)
            })
                )
            );
        }
    };
    // Create the JSON value
    let json_value = serde_json::json!({ "token": token });
    // Convert JSON to a string
    let json_string = serde_json::to_string(&json_value)?;
    ok_response(json_string).await
}

#[allow(dead_code)]
fn validate_token(token: &str) -> bool {
    let validation = Validation::new(Algorithm::HS256);
    decode::<Claims>(token, &DecodingKey::from_secret("secret".as_ref()), &validation).is_ok()
}

#[allow(dead_code)]
// Helper function for token validation
fn validate_auth_token(req: &HttpRequest) -> Result<(), &'static str> {
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        .ok_or("Missing Authorization token")?;

    if validate_token(token) {
        Ok(())
    } else {
        Err("Invalid or expired token")
    }
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
    query: web::Query<MAVLinkHelperQuery>
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
                    parse_query(
                        &(data::MAVLinkMessage {
                            header: mavlink::MavHeader::default(),
                            message: msg,
                        })
                    )
                }
                msg =>
                    parse_query(
                        &(data::MAVLinkMessage {
                            header: mavlink::MavHeader::default(),
                            message: msg,
                        })
                    ),
            };

            ok_response(msg).await
        }
        Err(content) => not_found_response(parse_query(&content)).await,
    }
}

#[api_v2_operation]
pub async fn fly_to(
    data: web::Data<MAVLinkVehicleArcMutex>,
    bytes: web::Bytes
) -> actix_web::Result<HttpResponse> {
    let target_location: TargetLocation = serde_json::from_slice(&bytes)?;

    let set_global_position_int_data = mavlink::common::MavMessage::SET_POSITION_TARGET_GLOBAL_INT(
        mavlink::common::SET_POSITION_TARGET_GLOBAL_INT_DATA {
            target_system: 1,
            target_component: 1,
            time_boot_ms: 0,
            coordinate_frame: mavlink::common::MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT_INT,
            type_mask: PositionTargetTypemask::empty() |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_VX_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_VY_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_VZ_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AX_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AY_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AZ_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_YAW_IGNORE |
            PositionTargetTypemask::POSITION_TARGET_TYPEMASK_YAW_RATE_IGNORE,
            lat_int: (target_location.lat * 1e7) as i32, // Latitude in degrees * 1e7
            lon_int: (target_location.lon * 1e7) as i32, // Longitude in degrees * 1e7
            alt: target_location.alt,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            afx: 0.0,
            afy: 0.0,
            afz: 0.0,
            yaw: 0.0,
            yaw_rate: 0.0,
        }
    );

    let set_global_position_int_msg = mavlink::ardupilotmega::MavMessage::common(
        set_global_position_int_data
    );
    match data.lock().unwrap().send(&mavlink::MavHeader::default(), &set_global_position_int_msg) {
        Ok(r) => { ok_response(format!("ack: {}", r)).await }
        Err(_) => todo!(),
    }
}

#[api_v2_operation]
pub async fn mission_clear(
    data: web::Data<MAVLinkVehicleArcMutex>,
    _req: HttpRequest
) -> actix_web::Result<HttpResponse> {
    let mission_clear_all = mavlink::common::MavMessage::MISSION_CLEAR_ALL(
        mavlink::common::MISSION_CLEAR_ALL_DATA {
            target_system: 1,
            target_component: 1,
            mission_type: mavlink::common::MavMissionType::MAV_MISSION_TYPE_MISSION,
        }
    );

    let mission_clear_all_data = mavlink::ardupilotmega::MavMessage::common(mission_clear_all);
    data.lock().unwrap().send(&mavlink::MavHeader::default(), &mission_clear_all_data)?;
    ok_response("mission clear sended".to_string()).await
}


#[api_v2_operation]
pub async fn set_fence(
    data: web::Data<MAVLinkVehicleArcMutex>,
    _req: HttpRequest
) -> actix_web::Result<HttpResponse> {
    // Define fence points (example coordinates)
    let fence_points = vec![
        (37.422, -122.0841), // Point 1
        (37.423, -122.0841), // Point 2
        (37.423, -122.0831), // Point 3
        (37.422, -122.0831) // Point 4
    ];

    for (index, (lat, lng)) in fence_points.iter().enumerate() {
        let fence_point = mavlink::ardupilotmega::MavMessage::FENCE_POINT(
            mavlink::ardupilotmega::FENCE_POINT_DATA {
                target_system: 1,
                target_component: 1,
                idx: index as u8,
                count: fence_points.len() as u8,
                lat: *lat,
                lng: *lng,
            }
        );

        data.lock().unwrap().send(&mavlink::MavHeader::default(), &fence_point)?;
    }

    ok_response("fence setup".to_string()).await
}

#[allow(dead_code)]
fn is_connected(data: &web::Data<MAVLinkVehicleArcMutex>) -> bool {
    // Attempt to lock the MAVLinkVehicle
    if let Ok(vehicle) = data.lock() {
        // Check if we've received any message recently
        // You might want to adjust the duration based on your requirements
        if let Ok(last_received) = vehicle.last_received() {
            return last_received.elapsed() < std::time::Duration::from_secs(5);
        }
    }
    // If we couldn't lock the vehicle or get the last received time, consider it disconnected
    false
}



#[api_v2_operation]
#[allow(clippy::await_holding_lock)]
/// Send a MAVLink message for the desired vehicle
pub async fn mavlink_post(
    data: web::Data<MAVLinkVehicleArcMutex>,
    _req: HttpRequest,
    bytes: web::Bytes
) -> actix_web::Result<HttpResponse> {
    if !is_connected(&data) {
        return ok_response("Please connect to vehicle first.".to_string()).await;
    }

    /* 
    validate_auth_token(&req).map_err(|e| {
        actix_web::error::ErrorUnauthorized(serde_json::json!({
            "error": e
        }))
    })?; */

    let json_string = match String::from_utf8(bytes.to_vec()) {
        Ok(content) => content,
        Err(err) => {
            return not_found_response(
                format!("Failed to parse input as UTF-8 string: {err:?}")
            ).await;
        }
    };

    debug!("MAVLink post received: {json_string}");

    if
        let Ok(content) =
            json5::from_str::<data::MAVLinkMessage<mavlink::ardupilotmega::MavMessage>>(
                &json_string
            )
    {
        match data.lock().unwrap().send(&content.header, &content.message) {
            Ok(_result) => {
                data::update((content.header, content.message));
                return HttpResponse::Ok().await;
            }
            Err(err) => {
                return not_found_response(format!("Failed to send message: {err:?}")).await;
            }
        }
    }
    not_found_response(format!("Failed to parsing message")).await
}

#[api_v2_operation]
/// Websocket used to receive and send MAVLink messages asynchronously
pub async fn websocket(
    req: HttpRequest,
    query: web::Query<WebsocketQuery>,
    stream: web::Payload
) -> Result<HttpResponse, actix_web::Error> {
    let filter = match query.into_inner().filter {
        Some(filter) => filter,
        _ => ".*".to_owned(),
    };

    debug!("New websocket with filter {:#?}", &filter);

    ws::start(WebsocketActor::new(filter), &req, stream)
}

async fn not_found_response(message: String) -> actix_web::Result<HttpResponse> {
    HttpResponse::NotFound().content_type("application/json").body(message).await
}

async fn ok_response(message: String) -> actix_web::Result<HttpResponse> {
    HttpResponse::Ok().content_type("application/json").body(message).await
}
