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

static MINIMUM_INPUT_FREQUENCY: u32 = 10;
static MAXIMUM_INPUT_FREQUENCY: u32 = 1000;
static MAXIMUM_POWER: u32 = 200;
static MAXIMUM_WAVEFORM_STRENGTH: u32 = 100;
static B0_HEAD: u8 = 0xB0;
#[allow(dead_code)]
static BF_HEAD: u8 = 0xBF;
static DEFAULT_SERIAL_NO: u8 = 0b0000;
#[allow(dead_code)]
static STRENGTH_PARSING_METHOD_NONE: u8 = 0b00;
#[allow(dead_code)]
static STRENGTH_PARSING_METHOD_INCREASE: u8 = 0b01;
#[allow(dead_code)]
static STRENGTH_PARSING_METHOD_DECREASE: u8 = 0b10;
static STRENGTH_PARSING_METHOD_SET_TO: u8 = 0b11;
static REPEAT_SLEEP_DURATION: u64 = 100;
static WAIT_UNTIL_TEST_DURATION: u64 = 500;

fn input_to_frequency(value: u32) -> u32 {
    match value {
        0 => value,
        10..=100 => value,
        101..=600 => (value - 100) / 5 + 100,
        601..=1000 => (value - 600) / 10 + 200,
        _ => 10,
    }
}

fn b0_set_command(
    power_a: u32,
    power_b: u32,
    frequency_a: [u32; 4],
    frequency_b: [u32; 4],
    waveform_strength_a: [u32; 4],
    waveform_strength_b: [u32; 4],
) -> Vec<u8> {
    let mut data: Vec<u8> = vec![
        B0_HEAD,
        (DEFAULT_SERIAL_NO << 4) | (STRENGTH_PARSING_METHOD_SET_TO << 2) | STRENGTH_PARSING_METHOD_SET_TO,
        power_a as u8,
        power_b as u8,
    ];
    data.extend(frequency_a.iter().map(|&x| x as u8));
    data.extend(waveform_strength_a.iter().map(|&x| x as u8));
    data.extend(frequency_b.iter().map(|&x| x as u8));
    data.extend(waveform_strength_b.iter().map(|&x| x as u8));
    return data;
}

fn b0_set_command_by_struct(dg_lab_v3: &DGLabV3) -> Vec<u8> {
    b0_set_command(
        dg_lab_v3.a_scalar.power.load(SeqCst),
        dg_lab_v3.b_scalar.power.load(SeqCst),
        [dg_lab_v3.a_scalar.frequency.load(SeqCst); 4],
        [dg_lab_v3.b_scalar.frequency.load(SeqCst); 4],
        [dg_lab_v3.a_scalar.waveform_strength.load(SeqCst); 4],
        [dg_lab_v3.b_scalar.waveform_strength.load(SeqCst); 4],
    )
}

struct ChannelScalar {
    power: Arc<AtomicU32>,
    frequency: Arc<AtomicU32>,
    waveform_strength: Arc<AtomicU32>,
}

generic_protocol_initializer_setup!(DGLabV3, "dg-lab-v3");

#[derive(Default)]
pub struct DGLabV3Initializer {}

#[async_trait]
impl ProtocolInitializer for DGLabV3Initializer {
    async fn initialize(
        &mut self,
        hardware: Arc<Hardware>,
        _: &ProtocolDeviceAttributes,
    ) -> Result<Arc<dyn ProtocolHandler>, ButtplugDeviceError> {
        let handler = Arc::new(DGLabV3::default());
        let handler_copy = handler.clone();
        let _ = async_manager::spawn(async move {
            let duration = Duration::from_millis(REPEAT_SLEEP_DURATION);
            // Wait until test finished, or it would cause failure of test (The order of HardwareCmd changed)
            // TODO: Maybe there's a better way to solve this
            util::sleep(Duration::from_millis(WAIT_UNTIL_TEST_DURATION)).await;
            loop {
                if let Err(e) = hardware.write_value(
                    &HardwareWriteCmd::new(
                        Endpoint::Tx,
                        b0_set_command_by_struct(&handler_copy),
                        false,
                    )
                ).await {
                    warn!("Error writing repeat packet: {:?}", e);
                }
                util::sleep(duration).await;
            }
        });
        Ok(handler)
    }
}

pub struct DGLabV3 {
    a_scalar: Arc<ChannelScalar>,
    b_scalar: Arc<ChannelScalar>,
}

impl Default for DGLabV3 {
    fn default() -> Self {
        Self {
            a_scalar: Arc::new(ChannelScalar {
                power: Arc::new(Default::default()),
                frequency: Arc::new(Default::default()),
                waveform_strength: Arc::new(Default::default()),
            }),
            b_scalar: Arc::new(ChannelScalar {
                power: Arc::new(Default::default()),
                frequency: Arc::new(Default::default()),
                waveform_strength: Arc::new(Default::default()),
            }),
        }
    }
}

impl ProtocolHandler for DGLabV3 {
    fn handle_scalar_cmd(&self, commands: &[Option<(ActuatorType, u32)>]) -> Result<Vec<HardwareCommand>, ButtplugDeviceError> {
        // Power A
        let power_a_scalar = self.a_scalar.power.clone();
        // Power B
        let power_b_scalar = self.b_scalar.power.clone();
        // Frequency A
        let frequency_a_scalar = self.a_scalar.frequency.clone();
        // Frequency B
        let frequency_b_scalar = self.b_scalar.frequency.clone();
        // Waveform strength A
        let waveform_strength_a_scalar = self.a_scalar.waveform_strength.clone();
        // Waveform strength B
        let waveform_strength_b_scalar = self.b_scalar.waveform_strength.clone();
        for (index, command) in commands.iter().enumerate().filter(|(_, x)| x.is_some()) {
            let (actuator, mut scalar) = command.as_ref().expect("Already verified existence");
            match *actuator {
                // Set power (S)
                ActuatorType::Vibrate => {
                    if scalar > MAXIMUM_POWER {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v3".to_owned(),
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
                                    "dg-lab-v3".to_owned(),
                                    format!("Vibrate command index {} is invalid", index),
                                )
                            );
                        }
                    }
                }
                // Set frequency (X, Y)
                ActuatorType::Oscillate => {
                    if scalar == MINIMUM_INPUT_FREQUENCY - 1 {
                        scalar = 0;
                    } else if scalar != 0 && (scalar < MINIMUM_INPUT_FREQUENCY || scalar > MAXIMUM_INPUT_FREQUENCY) {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v3".to_owned(),
                                format!("Frequency scalar {} not in [{}, {}]", scalar, MINIMUM_INPUT_FREQUENCY, MAXIMUM_INPUT_FREQUENCY),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        2 => { frequency_a_scalar.store(input_to_frequency(scalar), SeqCst); }
                        // Channel B
                        3 => { frequency_b_scalar.store(input_to_frequency(scalar), SeqCst); }
                        _ => {
                            return Err(
                                ProtocolSpecificError(
                                    "dg-lab-v3".to_owned(),
                                    format!("Oscillate command index {} is invalid", index),
                                )
                            );
                        }
                    }
                }
                // Set waveform strength (Z)
                ActuatorType::Inflate => {
                    if scalar > MAXIMUM_WAVEFORM_STRENGTH {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v3".to_owned(),
                                format!("Waveform strength scalar {} not in [0, {}]", scalar, MAXIMUM_WAVEFORM_STRENGTH),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        4 => { waveform_strength_a_scalar.store(scalar, SeqCst); }
                        // Channel B
                        5 => { waveform_strength_b_scalar.store(scalar, SeqCst); }
                        _ => {
                            return Err(
                                ProtocolSpecificError(
                                    "dg-lab-v3".to_owned(),
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
            vec![
                HardwareWriteCmd::new(
                    Endpoint::Tx,
                    b0_set_command_by_struct(self),
                    false,
                ).into(),
            ]
        )
    }
}