// Buttplug Rust Source Code File - See https://buttplug.io for more info.
//
// Copyright 2016-2023 Nonpolynomial Labs LLC. All rights reserved.
//
// Licensed under the BSD 3-Clause license. See LICENSE file in the project root
// for full license information.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use async_trait::async_trait;

use crate::{core::errors::ButtplugDeviceError, generic_protocol_initializer_setup, server::device::protocol::ProtocolHandler, util};
use crate::core::errors::ButtplugDeviceError::ProtocolSpecificError;
use crate::core::message::{ActuatorType, Endpoint};
use crate::server::device::configuration::ProtocolDeviceAttributes;
use crate::server::device::hardware::{Hardware, HardwareCommand, HardwareWriteCmd};
use crate::server::device::protocol::ProtocolAttributesType;
use crate::server::device::protocol::ProtocolIdentifier;
use crate::server::device::protocol::ProtocolInitializer;
use crate::server::ServerDeviceIdentifier;
use crate::util::async_manager;

static MINIMUM_FREQUENCY: u32 = 10;
static MAXIMUM_FREQUENCY: u32 = 1000;
static MAXIMUM_POWER: u32 = 2047;
static MAXIMUM_PULSE_WIDTH: u32 = 31;
static MAXIMUM_X: f32 = 31f32;
static MAXIMUM_Y: f32 = 1023f32;
static REPEAT_SLEEP_DURATION: u64 = 100;
static WAIT_UNTIL_TEST_DURATION: u64 = 500;


/// AAAA AAAA AAAB BBBB BBBB BB00
fn ab_power_to_byte(a: u32, b: u32) -> Vec<u8> {
    let data = 0 | ((b & 0x7FF) << 11) | (a & 0x7FF);
    return vec![
        (data & 0xFF) as u8,
        ((data >> 8) & 0xFF) as u8,
        ((data >> 16) & 0xFF) as u8,
    ];
}

fn frequency_to_xy(frequency: u32) -> (u32, u32) {
    let mut x = (frequency as f32 / 1000f32).sqrt() * 15f32;
    let mut y = frequency as f32 - x;
    if x > MAXIMUM_X { x = MAXIMUM_X }
    if y > MAXIMUM_Y { y = MAXIMUM_Y }
    return (x.round() as u32, y.round() as u32);
}

/// XXXX XYYY YYYY YYYZ ZZZZ 0000
fn xyz_to_bytes(x: u32, y: u32, z: u32) -> Vec<u8> {
    let data = 0 | ((z & 0x1F) << 15) | ((y & 0x3FF) << 5) | (x & 0x1F);
    return vec![
        (data & 0xFF) as u8,
        ((data >> 8) & 0xFF) as u8,
        ((data >> 16) & 0xFF) as u8,
    ];
}

fn commands_vec_by_struct(dg_lab_v2: &DGLabV2) -> Vec<HardwareWriteCmd> {
    vec![
        HardwareWriteCmd::new(
            Endpoint::Tx,
            ab_power_to_byte(
                dg_lab_v2.a_scalar.power.load(SeqCst),
                dg_lab_v2.b_scalar.power.load(SeqCst),
            ),
            false,
        ),
        HardwareWriteCmd::new(
            Endpoint::Generic0,
            xyz_to_bytes(
                dg_lab_v2.a_scalar.xy.0.load(SeqCst),
                dg_lab_v2.a_scalar.xy.1.load(SeqCst),
                dg_lab_v2.a_scalar.pulse_width.load(SeqCst),
            ),
            false,
        ),
        HardwareWriteCmd::new(
            Endpoint::Generic1,
            xyz_to_bytes(
                dg_lab_v2.b_scalar.xy.0.load(SeqCst),
                dg_lab_v2.b_scalar.xy.1.load(SeqCst),
                dg_lab_v2.b_scalar.pulse_width.load(SeqCst),
            ),
            false,
        ),
    ]
}

#[derive(Default)]
struct ChannelScalar {
    power: Arc<AtomicU32>,
    xy: Arc<(AtomicU32, AtomicU32)>,
    pulse_width: Arc<AtomicU32>,
}

#[derive(Default)]
pub struct DGLabV2 {
    a_scalar: Arc<ChannelScalar>,
    b_scalar: Arc<ChannelScalar>,
}

generic_protocol_initializer_setup!(DGLabV2, "dg-lab-v2");

#[derive(Default)]
pub struct DGLabV2Initializer {}

#[async_trait]
impl ProtocolInitializer for DGLabV2Initializer {
    async fn initialize(
        &mut self,
        hardware: Arc<Hardware>,
        _: &ProtocolDeviceAttributes,
    ) -> Result<Arc<dyn ProtocolHandler>, ButtplugDeviceError> {
        let handler = Arc::new(DGLabV2::default());
        let handler_copy = handler.clone();
        let _ = async_manager::spawn(async move {
            let duration = Duration::from_millis(REPEAT_SLEEP_DURATION);
            // Wait until test finished, or it would cause failure of test (The order of HardwareCmd changed)
            // TODO: Maybe there's a better way to solve this
            util::sleep(Duration::from_millis(WAIT_UNTIL_TEST_DURATION)).await;
            loop {
                for cmd in &commands_vec_by_struct(&handler_copy)[1..] {
                    if let Err(e) = hardware.write_value(&cmd).await {
                        warn!("Error writing repeat packet: {:?}", e);
                    }
                }
                util::sleep(duration).await;
            }
        });
        Ok(handler)
    }
}

impl ProtocolHandler for DGLabV2 {
    fn handle_scalar_cmd(&self, commands: &[Option<(ActuatorType, u32)>]) -> Result<Vec<HardwareCommand>, ButtplugDeviceError> {
        // Power A (S)
        let power_a_scalar = self.a_scalar.power.clone();
        // Power B (S)
        let power_b_scalar = self.b_scalar.power.clone();
        // Frequency A (X, Y)
        let xy_a_scalar = self.a_scalar.xy.clone();
        // Frequency B (X, Y)
        let xy_b_scalar = self.b_scalar.xy.clone();
        // Pulse width A (Z)
        let pulse_width_a_scalar = self.a_scalar.pulse_width.clone();
        // Pulse width B (Z)
        let pulse_width_b_scalar = self.b_scalar.pulse_width.clone();
        for (index, command) in commands.iter().enumerate().filter(|(_, x)| x.is_some()) {
            let (actuator, mut scalar) = command.as_ref().expect("Already verified existence");
            match *actuator {
                // Set power (S)
                ActuatorType::Vibrate => {
                    if scalar > MAXIMUM_POWER {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v2".to_owned(),
                                format!("Power scalar {} not in [0, {}]", scalar, MAXIMUM_POWER),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        0 => { power_a_scalar.store(scalar, SeqCst); }
                        // Channel B
                        1 => { power_b_scalar.store(scalar, SeqCst); }
                        _ => {
                            return Err(
                                ProtocolSpecificError(
                                    "dg-lab-v2".to_owned(),
                                    format!("Vibrate command index {} is invalid", index),
                                )
                            );
                        }
                    }
                }
                // Set frequency (X, Y)
                ActuatorType::Oscillate => {
                    if scalar == MINIMUM_FREQUENCY - 1 {
                        scalar = 0
                    } else if scalar != 0 && (scalar < MINIMUM_FREQUENCY || scalar > MAXIMUM_FREQUENCY) {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v2".to_owned(),
                                format!("Frequency scalar {} not in [{}, {}]", scalar, MINIMUM_FREQUENCY, MAXIMUM_FREQUENCY),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        2 => {
                            let (x_scalar, y_scalar) = frequency_to_xy(scalar);
                            xy_a_scalar.0.store(x_scalar, SeqCst);
                            xy_a_scalar.1.store(y_scalar, SeqCst);
                        }
                        // Channel B
                        3 => {
                            let (x_scalar, y_scalar) = frequency_to_xy(scalar);
                            xy_b_scalar.0.store(x_scalar, SeqCst);
                            xy_b_scalar.1.store(y_scalar, SeqCst);
                        }
                        _ => {
                            return Err(
                                ProtocolSpecificError(
                                    "dg-lab-v2".to_owned(),
                                    format!("Oscillate command index {} is invalid", index),
                                )
                            );
                        }
                    }
                }
                // Set pulse width (Z)
                ActuatorType::Inflate => {
                    if scalar > MAXIMUM_PULSE_WIDTH {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v2".to_owned(),
                                format!("Pulse width scalar {} not in [0, {}]", scalar, MAXIMUM_PULSE_WIDTH),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        4 => { pulse_width_a_scalar.store(scalar, SeqCst); }
                        // Channel B
                        5 => { pulse_width_b_scalar.store(scalar, SeqCst); }
                        _ => {
                            return Err(
                                ProtocolSpecificError(
                                    "dg-lab-v2".to_owned(),
                                    format!("Inflate command index {} is invalid", index),
                                )
                            );
                        }
                    }
                }
                _ => {
                    return Err(ButtplugDeviceError::UnhandledCommand(
                        "Unknown actuator types are not controllable.".to_owned(),
                    ));
                }
            }
        }
        Ok(
            commands_vec_by_struct(self)
                .into_iter()
                .map(|cmd| HardwareCommand::from(cmd))
                .collect()
        )
    }
}