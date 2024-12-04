mod cli;
mod data;
mod endpoints;
mod mavlink_vehicle;
mod server;
mod websocket_manager;
mod group;

use std::{ sync::{ Arc, Mutex }, thread };

use data::MAVLinkMessage;
use log::*;

fn main() -> std::io::Result<()> {
    let log_filter = if cli::is_verbose() { "debug" } else { "warn" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_filter)).init();
    cli::init();

    let mavlink_version = match cli::mavlink_version() {
        1 => mavlink::MavlinkVersion::V1,
        2 => mavlink::MavlinkVersion::V2,
        _ => panic!("Invalid mavlink version."),
    };

    let (system_id, component_id) = cli::mavlink_system_and_component_id();
    let connection_strings = cli::mavlink_connection_string();
    let server_addresses = cli::server_address();

    if connection_strings.len() != server_addresses.len() {
        panic!("The number of connection strings must match the number of server addresses.");
    }

    for (conn_string, server_address) in connection_strings
        .into_iter()
        .zip(server_addresses.into_iter()) {
        let vehicle =
            mavlink_vehicle::MAVLinkVehicleHandle::<mavlink::ardupilotmega::MavMessage>::new(
                &conn_string,
                mavlink_version,
                system_id,
                component_id,
                cli::mavlink_send_initial_heartbeats()
            );

        let inner_vehicle = vehicle.mavlink_vehicle.clone();

        // Clone for the server thread
        let vehicle_clone = Arc::clone(&inner_vehicle);

        // Spawn a new thread for each server
        thread::spawn(move || {
            server::run(&server_address, &vehicle_clone);
        });

        // Clone for the websocket manager
        let vehicle_clone = Arc::clone(&inner_vehicle);

        // Set up websocket callback for this vehicle
        websocket_manager::manager().lock().unwrap().new_message_callback = Some(
            Arc::new(move |value| { ws_callback(vehicle_clone.clone(), value) })
        );

        // Spawn a thread to handle messages for this vehicle
        thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));

                // let mut vehicle = vehicle_arc.lock().unwrap();
                while let Ok((header, message)) = vehicle.thread_rx_channel.try_recv() {
                    debug!("Received: {:#?} {:#?}", header, message);
                    websocket_manager::send(
                        &(MAVLinkMessage {
                            header,
                            message: message.clone(),
                        })
                    );
                    data::update((header, message));
                }
            }
        });
    }

    // Keep the main thread alive
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}

fn ws_callback(
    inner_vehicle: Arc<Mutex<mavlink_vehicle::MAVLinkVehicle<mavlink::ardupilotmega::MavMessage>>>,
    value: &str
) -> String {
    if
        let Ok(content @ MAVLinkMessage::<mavlink::ardupilotmega::MavMessage> { .. }) =
            serde_json::from_str(value)
    {
        let result = inner_vehicle.lock().unwrap().send(&content.header, &content.message);
        if result.is_ok() {
            data::update((content.header, content.message));
        }

        format!("{result:?}")
    } else if
        let Ok(content @ MAVLinkMessage::<mavlink::common::MavMessage> { .. }) =
            serde_json::from_str(value)
    {
        let content_ardupilotmega = mavlink::ardupilotmega::MavMessage::common(content.message);
        let result = inner_vehicle.lock().unwrap().send(&content.header, &content_ardupilotmega);
        if result.is_ok() {
            data::update((content.header, content_ardupilotmega));
        }

        format!("{result:?}")
    } else {
        String::from("Could not convert input message.")
    }
}
