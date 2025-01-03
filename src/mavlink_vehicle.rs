use std::sync::{ mpsc, Arc, Mutex };
use log::*;
use mavlink::Message;

type AckCallback = Box<dyn FnMut(mavlink::common::MavMessage) + Send>;
pub type MAVLinkVehicleArcMutex = Arc<Mutex<MAVLinkVehicle<mavlink::ardupilotmega::MavMessage>>>;

#[derive(Clone)]
pub struct MAVLinkVehicle<M: mavlink::Message> {
    //TODO: Check if Arc<Box can be only Arc or Box
    vehicle: Arc<Box<dyn mavlink::MavConnection<M> + Sync + Send>>,
    header: Arc<Mutex<mavlink::MavHeader>>,
    ack_callback: Arc<Mutex<Option<AckCallback>>>,
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

    pub fn set_ack_callback<F>(&self, callback: F)
    where
        F: FnMut(mavlink::common::MavMessage) + Send + 'static,
    {
        if let Ok(mut ack_callback) = self.ack_callback.lock() {
            *ack_callback = Some(Box::new(callback));
        }
    }

    pub fn clear_ack_callback(&self) {
        if let Ok(mut ack_callback) = self.ack_callback.lock() {
            *ack_callback = None;
        }
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
        component_id: u8
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
            ack_callback: Arc::new(Mutex::new(None)),
        }
    }
}

impl<
    M: 'static + mavlink::Message + std::fmt::Debug + From<mavlink::common::MavMessage> + Send
> MAVLinkVehicleHandle<M> {
    pub fn new(
        connection_string: &str,
        version: mavlink::MavlinkVersion,
        system_id: u8,
        component_id: u8,
        send_initial_heartbeats: bool
    ) -> Self {
        let mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>> = Arc::new(
            Mutex::new(
                MAVLinkVehicle::<M>::new(connection_string, version, system_id, component_id)
            )
        );

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
                receive_message_loop(receive_message_mavlink_vehicle, tx_channel);
            }),
            thread_rx_channel: rx_channel,
        }
    }
}

fn receive_message_loop<M: mavlink::Message + std::fmt::Debug + From<mavlink::common::MavMessage>>(
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>,
    channel: std::sync::mpsc::Sender<(mavlink::MavHeader, M)>
) {
    let vehicle = {
        let mavlink_vehicle = mavlink_vehicle.lock().unwrap();
        mavlink_vehicle.vehicle.clone()
    };

    let ack_callback = {
        let mavlink_vehicle = mavlink_vehicle.lock().unwrap();
        mavlink_vehicle.ack_callback.clone()
    };

    loop {
        match vehicle.recv() {
            Ok((header, msg)) => {
                if let Ok(parsed_message) = mavlink::common::MavMessage::parse(
                    mavlink::MavlinkVersion::V2, 
                    msg.message_id(), 
                    &msg.ser()
                ) {
                    match mavlink::ardupilotmega::MavMessage::common(parsed_message.clone()) {
                        mavlink::ardupilotmega::MavMessage::common(message) => {
                            match message {
                                mavlink::common::MavMessage::COMMAND_ACK(_) |
                                mavlink::common::MavMessage::HOME_POSITION(_)|
                                mavlink::common::MavMessage::MISSION_ACK(_) |
                                mavlink::common::MavMessage::MISSION_REQUEST(_) |
                                mavlink::common::MavMessage::MISSION_COUNT(_) |
                                mavlink::common::MavMessage::MISSION_ITEM_INT(_) => {
                                    // Call the callback if one is set
                                    if let Ok(mut ack_callback) = ack_callback.lock() {
                                        if let Some(callback) = ack_callback.as_mut() {
                                            callback(message.clone());
                                        }
                                    }
                                    info!("Received acknowledgment message: {:?}", message);
                                }
                                _ => {
                                    trace!("Received unhandled message type: {}", message.message_name());
                                }
                            }
                        }
                        _ => {
                            trace!("Received uncoverd message type");
                        }
                    }
                }

                if let Err(error) = channel.send((header, msg)) {
                    error!("Failed to send message through channel: {:#?}", error);
                }
            }
            Err(error) => {
                error!("Recv error: {:?}", error);
                if let mavlink::error::MessageReadError::Io(error) = error {
                    if error.kind() == std::io::ErrorKind::UnexpectedEof {
                        std::process::exit(0);
                    }
                }
            }
        }
    }
}

fn send_heartbeat<M: mavlink::Message + From<mavlink::common::MavMessage>>(
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>
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
    mavlink_vehicle: Arc<Mutex<MAVLinkVehicle<M>>>
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
