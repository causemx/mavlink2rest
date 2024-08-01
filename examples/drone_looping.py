from pymavlink import mavutil
from pymavlink.mavutil import mavlink
import time

def connect_vehicle():
    # Connect to the Vehicle
    print("Connecting to vehicle...")
    connection_string = 'udp:127.0.0.1:14550'
    vehicle = mavutil.mavlink_connection(connection_string)

    # Wait for the first heartbeat
    vehicle.wait_heartbeat()
    print("Vehicle connected!")
    return vehicle

def arm_and_takeoff(vehicle, altitude):
    # Arm vehicle
    vehicle.mav.command_long_send(
        vehicle.target_system, vehicle.target_component,
        mavlink.MAV_CMD_COMPONENT_ARM_DISARM, 0,
        1, 0, 0, 0, 0, 0, 0)

    # Wait for arming
    while True:
        if vehicle.motors_armed():
            print("Armed!")
            break
        time.sleep(1)

    # Takeoff
    vehicle.mav.command_long_send(
        vehicle.target_system, vehicle.target_component,
        mavlink.MAV_CMD_NAV_TAKEOFF, 0,
        0, 0, 0, 0, 0, 0, altitude)

    print(f"Taking off to altitude {altitude}m")

def execute_mission(vehicle):
    # Start mission
    vehicle.mav.command_long_send(
        vehicle.target_system, vehicle.target_component,
        mavlink.MAV_CMD_MISSION_START, 0,
        0, 0, 0, 0, 0, 0, 0)

    print("Mission started")

    # Wait for mission to complete
    while True:
        msg = vehicle.recv_match(type=['MISSION_CURRENT', 'MISSION_ITEM_REACHED'], blocking=True)
        if msg.get_type() == 'MISSION_ITEM_REACHED':
            if msg.seq == vehicle.mission_item_count - 1:  # Last waypoint reached
                print("Mission completed")
                break

def return_to_launch(vehicle):
    vehicle.mav.command_long_send(
        vehicle.target_system, vehicle.target_component,
        mavlink.MAV_CMD_NAV_RETURN_TO_LAUNCH, 0,
        0, 0, 0, 0, 0, 0, 0)

    print("Returning to launch")

    # Wait for RTL to complete
    while True:
        if vehicle.location().alt < 1:  # Assuming altitude less than 1m means landed
            print("Landed")
            break
        time.sleep(1)

def main():
    vehicle = connect_vehicle()

    for i in range(10):
        print(f"\nStarting mission {i+1}")
        arm_and_takeoff(vehicle, 10)  # Take off to 10m altitude
        execute_mission(vehicle)
        return_to_launch(vehicle)
        print(f"Mission {i+1} completed")

    print("\nAll 10 missions completed!")

if __name__ == "__main__":
    main()
