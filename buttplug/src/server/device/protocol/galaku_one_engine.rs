// Buttplug Rust Source Code File - See https://buttplug.io for more info.
//
// Copyright 2016-2023 Nonpolynomial Labs LLC. All rights reserved.
//
// Licensed under the BSD 3-Clause license. See LICENSE file in the project root
// for full license information.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{future, FutureExt};
use futures_util::future::BoxFuture;
use tokio::time::sleep;

use crate::{core::{errors::ButtplugDeviceError, message::Endpoint}, generic_protocol_initializer_setup, server::device::{
    hardware::{HardwareCommand, HardwareWriteCmd},
    protocol::ProtocolHandler,
}};
use crate::core::message;
use crate::core::message::{ButtplugDeviceMessage, ButtplugMessage, ButtplugServerMessage, LinearCmd, SensorReadCmd, SensorReading, SensorSubscribeCmd, SensorType, SensorUnsubscribeCmd};
use crate::server::device::configuration::ProtocolDeviceAttributes;
use crate::server::device::hardware::{Hardware, HardwareEvent, HardwareSubscribeCmd, HardwareUnsubscribeCmd};
use crate::server::device::protocol::ProtocolAttributesType;
use crate::server::device::protocol::ProtocolIdentifier;
use crate::server::device::protocol::ProtocolInitializer;
use crate::server::ServerDeviceIdentifier;

static KEY_TAB: [[u32; 12]; 4] = [
    [0, 24, 152, 247, 165, 61, 13, 41, 37, 80, 68, 70],
    [0, 69, 110, 106, 111, 120, 32, 83, 45, 49, 46, 55],
    [0, 101, 120, 32, 84, 111, 121, 115, 10, 142, 157, 163],
    [0, 197, 214, 231, 248, 10, 50, 32, 111, 98, 13, 10]
];

static LINEAR_CONTROL_INTERVAL: u32 = 100;

fn get_tab_key(r: usize, t: usize) -> u32 {
    let e = 3 & r;
    return KEY_TAB[e][t];
}

fn encrypt(data: Vec<u32>) -> Vec<u32> {
    let mut new_data = vec![data[0]];
    for i in 1..data.len() {
        let a = get_tab_key(new_data[i - 1] as usize, i);
        let u = (a ^ data[0] ^ data[i]) + a;
        new_data.push(u);
    }
    return new_data;
}

fn decrypt(data: Vec<u32>) -> Vec<u32> {
    let mut new_data = vec![data[0]];
    for i in 1..data.len() {
        let a = get_tab_key(data[i - 1] as usize, i);
        let u = data[i] as i32 - a as i32 ^ data[0] as i32 ^ a as i32;
        new_data.push(if u < 0 { (u + 256) as u32 } else { u as u32 });
    }
    return new_data;
}

fn send_bytes(data: Vec<u32>) -> Vec<u8> {
    let mut new_data = vec![35];
    new_data.extend(data);
    new_data.push(new_data.iter().sum());
    let mut uint8_array: Vec<u8> = Vec::new();
    for value in encrypt(new_data) {
        uint8_array.push(value as u8);
    }
    return uint8_array;
}

fn read_value(data: Vec<u8>) -> u32 {
    let mut uint32_data: Vec<u32> = Vec::new();
    for value in data {
        uint32_data.push(value as u32);
    }
    let decrypted_data = decrypt(uint32_data);
    if decrypted_data.len() > 0 {
        decrypted_data[4]
    } else {
        0
    }
}

fn vibrate_data(scalar: u32) -> Vec<u8> {
    let data: Vec<u32> = vec![90, 0, 0, 1, 49, scalar, 0, 0, 0, 0];
    return send_bytes(data);
}

fn vibrate_command(scalar: u32) -> HardwareWriteCmd {
    return HardwareWriteCmd::new(Endpoint::Tx, vibrate_data(scalar), false);
}

generic_protocol_initializer_setup!(GalakuOneEngine, "galaku-one-engine");

#[derive(Default)]
pub struct GalakuOneEngineInitializer {}

#[async_trait]
impl ProtocolInitializer for GalakuOneEngineInitializer {
    async fn initialize(&mut self, hardware: Arc<Hardware>, _attributes: &ProtocolDeviceAttributes) -> Result<Arc<dyn ProtocolHandler>, ButtplugDeviceError> {
        Ok(Arc::new(GalakuOneEngine::new(hardware)))
    }
}

pub struct GalakuOneEngine {
    hardware: Arc<Hardware>,
    vibrate_value: Arc<Mutex<u32>>,
}

impl GalakuOneEngine {
    pub fn new(hardware: Arc<Hardware>) -> Self {
        Self { hardware, vibrate_value: Arc::new(Mutex::new(0)) }
    }
}

impl ProtocolHandler for GalakuOneEngine {
    fn keepalive_strategy(&self) -> super::ProtocolKeepaliveStrategy {
        super::ProtocolKeepaliveStrategy::RepeatLastPacketStrategy
    }

    fn handle_scalar_vibrate_cmd(&self, _index: u32, scalar: u32) -> Result<Vec<HardwareCommand>, ButtplugDeviceError> {
        let mut vibrate_value = self.vibrate_value.lock().unwrap();
        *vibrate_value = scalar;
        Ok(vec![vibrate_command(scalar).into()])
    }

    fn handle_linear_cmd(&self, message: LinearCmd) -> Result<Vec<HardwareCommand>, ButtplugDeviceError> {
        let mut vibrate_value = self.vibrate_value.lock().unwrap();
        let mut commands = vec![];
        for vector in message.vectors() {
            let step_num = vector.duration() / LINEAR_CONTROL_INTERVAL;
            let step = ((vector.position() * 100f64 - *vibrate_value as f64) / step_num as f64).round() as i32;
            for _ in 0..step_num {
                commands.push(
                    HardwareCommand::Write(vibrate_command((*vibrate_value as i32 + step) as u32))
                );
            }
            *vibrate_value = (vector.position() * 100f64) as u32;
        }
        return Ok(commands.into())
    }

    fn handle_sensor_subscribe_cmd(&self, device: Arc<Hardware>, message: SensorSubscribeCmd) -> BoxFuture<Result<ButtplugServerMessage, ButtplugDeviceError>> {
        match message.sensor_type() {
            SensorType::Battery => {
                async move {
                    device.subscribe(&HardwareSubscribeCmd::new(Endpoint::RxBLEBattery)).await?;
                    Ok(message::Ok::new(message.id()).into())
                }
            }
                .boxed(),
            _ => future::ready(Err(ButtplugDeviceError::UnhandledCommand(
                "Command not implemented for this sensor".to_string(),
            )))
                .boxed(),
        }
    }

    fn handle_sensor_unsubscribe_cmd(&self, device: Arc<Hardware>, message: SensorUnsubscribeCmd) -> BoxFuture<Result<ButtplugServerMessage, ButtplugDeviceError>> {
        match message.sensor_type() {
            SensorType::Battery => {
                async move {
                    device.unsubscribe(&HardwareUnsubscribeCmd::new(Endpoint::RxBLEBattery)).await?;
                    Ok(message::Ok::new(message.id()).into())
                }
            }
                .boxed(),
            _ => future::ready(Err(ButtplugDeviceError::UnhandledCommand(
                "Command not implemented for this sensor".to_string(),
            )))
                .boxed(),
        }
    }

    fn handle_battery_level_cmd(&self, device: Arc<Hardware>, message: SensorReadCmd) -> BoxFuture<Result<ButtplugServerMessage, ButtplugDeviceError>> {
        let data: Vec<u32> = vec![90, 0, 0, 1, 19, 0, 0, 0, 0, 0];
        let mut device_notification_receiver = device.event_stream();
        async move {
            device.subscribe(&HardwareSubscribeCmd::new(Endpoint::RxBLEBattery)).await?;
            device.write_value(&HardwareWriteCmd::new(Endpoint::Tx, send_bytes(data), true)).await?;
            while let Ok(event) = device_notification_receiver.recv().await {
                return match event {
                    HardwareEvent::Notification(_, endpoint, data) => {
                        if endpoint != Endpoint::RxBLEBattery {
                            continue;
                        }
                        let battery_reading = SensorReading::new(
                            message.device_index(),
                            *message.sensor_index(),
                            *message.sensor_type(),
                            vec![read_value(data) as i32],
                        );
                        Ok(battery_reading.into())
                    }
                    HardwareEvent::Disconnected(_) => {
                        Err(ButtplugDeviceError::ProtocolSpecificError(
                            "GalakuOneEngine".to_owned(),
                            "Galaku Device disconnected while getting Battery info.".to_owned(),
                        ))
                    }
                };
            }
            Err(ButtplugDeviceError::ProtocolSpecificError(
                "GalakuOneEngine".to_owned(),
                "Galaku Device disconnected while getting Battery info.".to_owned(),
            ))
        }
            .boxed()
    }
}
