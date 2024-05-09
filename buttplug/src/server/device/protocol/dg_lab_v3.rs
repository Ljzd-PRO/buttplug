// Buttplug Rust Source Code File - See https://buttplug.io for more info.
//
// Copyright 2016-2023 Nonpolynomial Labs LLC. All rights reserved.
//
// Licensed under the BSD 3-Clause license. See LICENSE file in the project root
// for full license information.

use crate::{core::errors::ButtplugDeviceError, server::device::protocol::{generic_protocol_setup, ProtocolHandler}};
use crate::core::errors::ButtplugDeviceError::ProtocolSpecificError;
use crate::core::message::{ActuatorType, Endpoint};
use crate::server::device::hardware::{HardwareCommand, HardwareWriteCmd};
use crate::server::device::protocol::ProtocolKeepaliveStrategy;

static MINIMUM_INPUT_FREQUENCY: u32 = 10;
static MAXIMUM_INPUT_FREQUENCY: u32 = 1000;
static MAXIMUM_POWER: u32 = 200;
static MAXIMUM_WAVEFORM_STRENGTH: u32 = 100;
static B0_HEAD: u8 = 0xB0;
#[warn(dead_code)]
static BF_HEAD: u8 = 0xBF;
#[warn(dead_code)]
static DEFAULT_SERIAL_NO: u8 = 0b0000;
static STRENGTH_PARSING_METHOD_NONE: u8 = 0b00;
static STRENGTH_PARSING_METHOD_INCREASE: u8 = 0b01;
static STRENGTH_PARSING_METHOD_DECREASE: u8 = 0b10;
static STRENGTH_PARSING_METHOD_SET_TO: u8 = 0b11;

generic_protocol_setup!(DGLabV3, "dg-lab-v3");

fn input_to_frequency(value: u32) -> u32 {
    match value {
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
        STRENGTH_PARSING_METHOD_SET_TO,
        power_a as u8,
        power_b as u8,
    ];
    data.extend(frequency_a.iter().map(|&x| x as u8));
    data.extend(frequency_b.iter().map(|&x| x as u8));
    data.extend(waveform_strength_a.iter().map(|&x| x as u8));
    data.extend(waveform_strength_b.iter().map(|&x| x as u8));
    return data;
}

#[derive(Default)]
pub struct DGLabV3 {}

impl ProtocolHandler for DGLabV3 {
    fn needs_full_command_set(&self) -> bool {
        true
    }

    fn keepalive_strategy(&self) -> ProtocolKeepaliveStrategy {
        ProtocolKeepaliveStrategy::RepeatLastPacketStrategy
    }

    fn handle_scalar_cmd(&self, commands: &[Option<(ActuatorType, u32)>]) -> Result<Vec<HardwareCommand>, ButtplugDeviceError> {
        // Power A
        let mut power_a_scalar: u32 = 0;
        // Power B
        let mut power_b_scalar: u32 = 0;
        // Frequency A
        let mut frequency_a_scalar: u32 = 0;
        // Frequency B
        let mut frequency_b_scalar: u32 = 0;
        // Waveform strength A
        let mut waveform_strength_a_scalar: u32 = 0;
        // Waveform strength B
        let mut waveform_strength_b_scalar: u32 = 0;
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
                        0 => { power_a_scalar = scalar; }
                        // Channel B
                        1 => { power_b_scalar = scalar; }
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
                    if scalar == 0 {
                        scalar = MINIMUM_INPUT_FREQUENCY;
                    } else if scalar < MINIMUM_INPUT_FREQUENCY || scalar > MAXIMUM_INPUT_FREQUENCY {
                        return Err(
                            ProtocolSpecificError(
                                "dg-lab-v3".to_owned(),
                                format!("Frequency scalar {} not in [{}, {}]", scalar, MINIMUM_INPUT_FREQUENCY, MAXIMUM_INPUT_FREQUENCY),
                            )
                        );
                    }
                    match index {
                        // Channel A
                        2 => { frequency_a_scalar = input_to_frequency(scalar); }
                        // Channel B
                        3 => { frequency_b_scalar = input_to_frequency(scalar); }
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
                        4 => { waveform_strength_a_scalar = scalar; }
                        // Channel B
                        5 => { waveform_strength_b_scalar = scalar; }
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
                    b0_set_command(
                        power_a_scalar,
                        power_b_scalar,
                        [frequency_a_scalar; 4],
                        [frequency_b_scalar; 4],
                        [waveform_strength_a_scalar; 4],
                        [waveform_strength_b_scalar; 4],
                    ),
                    false,
                ).into(),
            ]
        )
    }
}