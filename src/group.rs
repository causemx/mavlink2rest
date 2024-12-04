use serde::{ Deserialize, Serialize };
use std::sync::{ Arc, Mutex };
use std::collections::HashMap;
use short_uuid::ShortUuid;
use paperclip::actix::Apiv2Schema;

#[derive(Debug, Clone, Serialize, Deserialize, Apiv2Schema)]
pub struct Vehicle {
    id: String,
    name: String,
    connection_string: String,
    system_id: u8,
    component_id: u8,
    is_leader: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Apiv2Schema)]
pub struct VehicleGroup {
    id: String,
    name: String,
    vehicles: HashMap<String, Vehicle>,
    leader_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Apiv2Schema)]
#[serde(deny_unknown_fields)]
pub struct CreateGroupRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, Apiv2Schema)]
pub struct UpdateGroupRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, Apiv2Schema)]
#[serde(deny_unknown_fields)]
pub struct AddVehicleRequest {
    pub name: String,
    pub connection_string: String,
    pub system_id: u8,
    pub component_id: u8,
}

#[derive(Debug, Deserialize, Apiv2Schema)]
#[serde(deny_unknown_fields)]
pub struct SetLeaderRequest {
    pub vehicle_id: String,
}

pub struct GroupManager {
    groups: HashMap<String, VehicleGroup>,
}

impl GroupManager {
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    pub fn create_group(&mut self, name: String) -> VehicleGroup {
        let group = VehicleGroup {
            id: ShortUuid::generate().to_string(),
            name,
            vehicles: HashMap::new(),
            leader_id: None,
        };
    
        self.groups.insert(group.id.clone(), group.clone());
        group

    }

    pub fn delete_group(&mut self, group_id: &str) -> Option<VehicleGroup> {
        self.groups.remove(group_id)
    }

    pub fn update_group(&mut self, group_id: &str, name: String) -> Option<VehicleGroup> {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.name = name;
            Some(group.clone())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn get_group(&self, group_id: &str) -> Option<&VehicleGroup> {
        self.groups.get(group_id)
    }

    pub fn list_groups(&self) -> Vec<&VehicleGroup> {
        self.groups.values().collect()
    }

    pub fn add_vehicle(
        &mut self,
        group_id: &str,
        name: String,
        connection_string: String,
        system_id: u8,
        component_id: u8
    ) -> Option<Vehicle> {
        let group = self.groups.get(group_id)?;
        // Check for duplicate system_id
        if group.vehicles.values().any(|v| v.system_id == system_id) {
            println!("Duplicated system id");
            return None;
        }

        if let Some(group) = self.groups.get_mut(group_id) {
            let is_first_vehicle = group.vehicles.is_empty();
            let vehicle = Vehicle {
                id: ShortUuid::generate().to_string(),
                name,
                connection_string,
                system_id,
                component_id,
                is_leader: is_first_vehicle, // First vehicle is automatically the leader
            };

            // If this is the first vehicle, set it as the leader
            if is_first_vehicle {
                group.leader_id = Some(vehicle.id.clone());
            }

            group.vehicles.insert(vehicle.id.clone(), vehicle.clone());
            Some(vehicle)
        } else {
            None
        }
    }

    pub fn remove_vehicle(&mut self, group_id: &str, vehicle_id: &str) -> Option<Vehicle> {
        if let Some(group) = self.groups.get_mut(group_id) {
            let removed_vehicle = group.vehicles.remove(vehicle_id);

            // If we removed the leader and there are other vehicles, assign a new leader
            if
                removed_vehicle
                    .as_ref()
                    .map(|v| v.is_leader)
                    .unwrap_or(false)
            {
                if !group.vehicles.is_empty() {
                    // Choose the first available vehicle as the new leader
                    if let Some((first_id, first_vehicle)) = group.vehicles.iter_mut().next() {
                        first_vehicle.is_leader = true;
                        group.leader_id = Some(first_id.clone());
                    }
                } else {
                    group.leader_id = None;
                }
            }

            removed_vehicle
        } else {
            None
        }
    }

    pub fn list_vehicles(&self, group_id: &str) -> Option<Vec<&Vehicle>> {
        self.groups.get(group_id).map(|group| group.vehicles.values().collect())
    }

    pub fn set_leader(&mut self, group_id: &str, vehicle_id: &str) -> Option<bool> {
        if let Some(group) = self.groups.get_mut(group_id) {
            // Check if the vehicle exists in the group
            if !group.vehicles.contains_key(vehicle_id) {
                return None;
            }

            // Remove leader status from current leader
            if let Some(current_leader_id) = &group.leader_id {
                if let Some(current_leader) = group.vehicles.get_mut(current_leader_id) {
                    current_leader.is_leader = false;
                }
            }

            // Set new leader
            if let Some(new_leader) = group.vehicles.get_mut(vehicle_id) {
                new_leader.is_leader = true;
                group.leader_id = Some(vehicle_id.to_string());
                return Some(true);
            }
        }
        None
    }

    pub fn get_leader(&self, group_id: &str) -> Option<&Vehicle> {
        self.groups
            .get(group_id)
            .and_then(|group| {
                group.leader_id.as_ref().and_then(|leader_id| group.vehicles.get(leader_id))
            })
    }
}

lazy_static::lazy_static! {
    pub static ref GROUP_MANAGER: Arc<Mutex<GroupManager>> = Arc::new(
        Mutex::new(GroupManager::new())
    );
}
