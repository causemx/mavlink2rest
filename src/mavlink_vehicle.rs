use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;
use log::*;
use mavlink::Message;

pub type MAVLinkVehicleArcMutex = Arc<Mutex<MAVLinkVehicle<mavlink::ardupilotmega::MavMessage>>>;

#[allow(dead_code)]
#[derive(Debug)]
pub enum MissionMessage {
    Count(u16),
    Item(mavlink::common::MISSION_ITEM_INT_DATA),
    Request(u16),
    Ack(mavlink::common::MavMissionResult),
    ParamValue(mavlink::common::PARAM_VALUE_DATA),
    CmdAck(mavlink::common::COMMAND_ACK_DATA),
}

#[derive(Clone)]
pub struct MAVLinkVehicle<M: mavlink::Message> {
    //TODO: Check if Arc<Box can be only Arc or Box
    vehicle: Arc<Box<dyn mavlink::MavConnection<M> + Sync + Send>>,
    header: Arc<Mutex<mavlink::MavHeader>>,
    last_message_time: Arc<Mutex<Instant>>,
}

impl<M: mavlink::Message> MAVLinkVehicle<M> {
    pub fn send(&self, header: &mavlink::MavHeader, message: &M) -> std::io::Result<usize> {
        let result = self.vehicle.send(header, message);

        // Convert from mavlink error to io error
        match result {
            Err(mavlink::error::MessageWriteError::Io(error)) => Err(error),
            Ok(something) => Ok(something),
        }
    }

    pub fn last_received(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Instant>> {
        self.last_message_time.lock()
    }
}

#[allow(dead_code)]
pub struct MAVLinkVehicleHandle<M: mavlink::Message> {
    //TODO: Check if we can use vehicle here directly
    pub mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>,
    heartbeat_thread: std::thread::JoinHandle<()>,
    receive_message_thread: std::thread::JoinHandle<()>,
    //TODO: Add a channel for errors
    pub thread_rx_channel: std::sync::mpsc::Receiver<(mavlink::MavHeader, M)>,
}

impl<M: mavlink::Message> MAVLinkVehicle<M> {
    fn new(
        mavlink_connection_string: &str,
        version: mavlink::MavlinkVersion,
        system_id: u8,
        component_id: u8,
    ) -> Self {
        let mut vehicle = mavlink::connect(mavlink_connection_string).unwrap();
        vehicle.set_protocol_version(version);
        let header = mavlink::MavHeader {
            system_id,
            component_id,
            sequence: 0,
        };

        Self {
            vehicle: Arc::new(vehicle),
            header: Arc::new(Mutex::new(header)),
            last_message_time: Arc::new(Mutex::new(Instant::now())),
        }
    }
}

impl<
        M: 'static + mavlink::Message + std::fmt::Debug + From<mavlink::common::MavMessage> + Send,
    > MAVLinkVehicleHandle<M>
{
    pub fn new(
        connection_string: &str,
        version: mavlink::MavlinkVersion,
        system_id: u8,
        component_id: u8,
        send_initial_heartbeats: bool,
    ) -> Self {
        let mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>> = Arc::new(Mutex::new(
            MAVLinkVehicle::<M>::new(connection_string, version, system_id, component_id),
        ));

        // PX4 requires a initial heartbeat to be sent to wake up the connection, otherwise it will
        // not send any messages
        if send_initial_heartbeats {
            // From testing, its better to wait a bit before sending the initial heartbeats since
            // when sending right away, some heartbeats are lost
            std::thread::sleep(std::time::Duration::from_secs(2));
            // Even though one heartbeat is enough, from testing seems like some times the first
            // heartbeat is lost, so send a small burst to make sure the connection one go through
            // and the connection is woken up
            for _ in 0..5 {
                send_heartbeat(mavlink_vehicle.clone());
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        let heartbeat_mavlink_vehicle = mavlink_vehicle.clone();
        let receive_message_mavlink_vehicle = mavlink_vehicle.clone();

        let (tx_channel, rx_channel) = mpsc::channel::<(mavlink::MavHeader, M)>();
        Self {
            mavlink_vehicle,
            heartbeat_thread: std::thread::spawn(move || heartbeat_loop(heartbeat_mavlink_vehicle)),
            receive_message_thread: std::thread::spawn(move || {
                receive_message_loop(
                    receive_message_mavlink_vehicle,
                    tx_channel,
                );
            }),
            thread_rx_channel: rx_channel,
        }
    }
}

fn receive_message_loop<
    M: mavlink::Message + std::fmt::Debug + From<mavlink::common::MavMessage>,
>(
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>,
    channel: std::sync::mpsc::Sender<(mavlink::MavHeader, M)>,
) {
    // let mavlink_vehicle = mavlink_vehicle.as_ref().lock().unwrap();
    // let vehicle = mavlink_vehicle.vehicle.clone();
    // drop(mavlink_vehicle);
    // let vehicle = vehicle.as_ref();

    let vehicle = {
        let mavlink_vehicle = mavlink_vehicle.lock().unwrap();
        mavlink_vehicle.vehicle.clone()
    };

    loop {
        match vehicle.recv() {
            Ok((header, msg)) => {
                // println!("id:{}, name:{}, msg: {:?}", msg.message_id(), msg.message_name(), msg);

                // println!("msg:{:?}", msg);

                 // Update the last_message_time
                 if let Ok(mavlink_vehicle) = mavlink_vehicle.lock() {
                    if let Ok(mut last_message_time) = mavlink_vehicle.last_message_time.lock() {
                        *last_message_time = Instant::now();
                    }
                }
                
                /* 
                let message_result = mavlink::common::MavMessage::parse(
                    mavlink::MavlinkVersion::V2,
                    msg.message_id(),
                    &msg.ser(),
                );

                // Then, handle the Result and match on the message type
                match message_result {
                    Ok(parsed_message) => {
                        match mavlink::ardupilotmega::MavMessage::common(parsed_message.clone()) {
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::MISSION_COUNT(count_data),
                            ) => {
                                println!("Got mission_count, count: {}", count_data.count);
                            }
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::MISSION_ITEM_INT(item_data),
                            ) => {
                                println!("Got mission_item_int, content: {:?}", item_data);
                            }
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::MISSION_REQUEST(request_data),
                            ) => {
                                println!("Got mission_request, seq: {}", request_data.seq);
                            }
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::MISSION_ACK(ack_data),
                            ) => {
                                println!("Got mission_ack, type: {:?}", ack_data.mavtype);
                            }
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::PARAM_VALUE(param_value)
                            ) => {
                                println!("Got param_value, param_id: {:?}", param_value.param_id);
                            }
                            mavlink::ardupilotmega::MavMessage::common(
                                mavlink::common::MavMessage::COMMAND_ACK(cmd_ack_data),
                            ) => {
                                println!("Got command_ack, data: {:?}", cmd_ack_data);
                            }
                            _ => {
                                // println!("Received unhandled message, msg:{}", parsed_message.message_name());
                            }
                        }
                    }
                    Err(_err) => {
                        // error!("Failed to parse MAVLink message: {:?}", err);
                        // Handle the error case
                    }
                }
                */

                if let Err(error) = channel.send((header, msg)) {
                    error!("Failed to send message though channel: {:#?}", error);
                }
            }
            Err(error) => {
                error!("Recv error: {:?}", error);
                if let mavlink::error::MessageReadError::Io(error) = error {
                    if error.kind() == std::io::ErrorKind::UnexpectedEof {
                        // We're probably running a file, time to exit!
                        std::process::exit(0);
                    };
                }
            }
        }
    }
}

fn send_heartbeat<M: mavlink::Message + From<mavlink::common::MavMessage>>(
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>,
) {
    let mavlink_vehicle = mavlink_vehicle.as_ref().lock().unwrap();
    let vehicle = mavlink_vehicle.vehicle.clone();
    let mut header = mavlink_vehicle.header.lock().unwrap();
    if let Err(error) = vehicle.as_ref().send(&header, &heartbeat_message().into()) {
        error!("Failed to send heartbeat: {:?}", error);
    }
    header.sequence = header.sequence.wrapping_add(1);
}

fn heartbeat_loop<M: mavlink::Message + From<mavlink::common::MavMessage>>(
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>,
) {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        send_heartbeat(mavlink_vehicle.clone());
    }
}

pub fn heartbeat_message() -> mavlink::common::MavMessage {
    mavlink::common::MavMessage::HEARTBEAT(mavlink::common::HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: mavlink::common::MavType::MAV_TYPE_ONBOARD_CONTROLLER,
        autopilot: mavlink::common::MavAutopilot::MAV_AUTOPILOT_INVALID,
        base_mode: mavlink::common::MavModeFlag::default(),
        system_status: mavlink::common::MavState::MAV_STATE_STANDBY,
        mavlink_version: 0x3,
    })
}
