//! HAL interface to the SAADC peripheral.
//!
//! Example usage:
//!
#![cfg_attr(feature = "52840", doc = "```no_run")]
#![cfg_attr(not(feature = "52840"), doc = "```ignore")]
//! # use nrf_hal_common as hal;
//! # use hal::pac::{saadc, SAADC};
//! // substitute `hal` with the HAL of your board, e.g. `nrf52840_hal`
//! use hal::{
//!    pac::Peripherals,
//!    prelude::*,
//!    gpio::p0::Parts as P0Parts,
//!    saadc::{SaadcConfig, Saadc},
//! };
//!
//! let board = Peripherals::take().unwrap();
//! let gpios = P0Parts::new(board.P0);
//!
//! // initialize saadc interface
//! let saadc_config = SaadcConfig::default();
//! let mut saadc = Saadc::new(board.SAADC, saadc_config);
//! let mut saadc_pin = gpios.p0_02; // the pin your analog device is connected to
//!
//! // blocking read from saadc for `saadc_config.time` microseconds
//! let _saadc_result = saadc.read_channel(&mut saadc_pin);
//! ```

#[cfg(any(feature = "9160", feature = "5340-app"))]
use crate::pac::{saadc_ns as saadc, SAADC_NS as SAADC};

#[cfg(not(any(feature = "9160", feature = "5340-app")))]
use crate::pac::{saadc, SAADC};

use core::future::Future;

pub use saadc::{
    ch::config::{GAIN_A as Gain, REFSEL_A as Reference, RESP_A as Resistor, TACQ_A as Time},
    oversample::OVERSAMPLE_A as Oversample,
    resolution::VAL_A as Resolution,
};

#[cfg(feature = "embedded-hal-02")]
pub trait Channel: embedded_hal_02::adc::Channel<Saadc, ID = u8> {}

#[cfg(not(feature = "embedded-hal-02"))]
pub trait Channel {
    const CHANNEL: u8;
}

// Only 1 channel is allowed right now, a discussion needs to be had as to how
// multiple channels should work (See "scan mode" in the datasheet).
// Issue: https://github.com/nrf-rs/nrf-hal/issues/82

/// Interface for the SAADC peripheral.
///
/// External analog channels supported by the SAADC implement the `Channel` trait.
/// Currently, use of only one channel is allowed.
pub struct Saadc(SAADC);

pub struct Resolver<'a, const N: usize> {
    saadc: &'a mut Saadc,
    buffer: [i16; N],
}

impl Saadc {
    pub fn new(saadc: SAADC, config: SaadcConfig) -> Self {
        // The write enums do not implement clone/copy/debug, only the
        // read ones, hence the need to pull out and move the values.
        let SaadcConfig {
            resolution,
            oversample,
            reference,
            gain,
            resistor,
            time,
        } = config;

        saadc.enable.write(|w| w.enable().enabled());
        saadc.resolution.write(|w| w.val().variant(resolution));
        saadc
            .oversample
            .write(|w| w.oversample().variant(oversample));
        saadc.samplerate.write(|w| w.mode().task());

        saadc.ch[0].config.write(|w| {
            w.refsel().variant(reference);
            w.gain().variant(gain);
            w.tacq().variant(time);
            w.mode().se();
            w.resp().variant(resistor);
            w.resn().bypass();
            w.burst().enabled();
            w
        });
        saadc.ch[0].pseln.write(|w| w.pseln().nc());

        // Calibrate
        saadc.events_calibratedone.reset();
        saadc.tasks_calibrateoffset.write(|w| unsafe { w.bits(1) });
        while saadc.events_calibratedone.read().bits() == 0 {}

        Saadc(saadc)
    }

    /// Disable SAADC and return the low-level peripheral handle
    pub fn free(self) -> SAADC {
        self.0.enable.write(|w| w.enable().disabled());
        self.0
    }

    /// Reads channels up until the specified index.
    ///
    /// NOTE: THIS IS VERY specialized for our usecase.
    pub fn read_channels<'a>(&'a mut self, channels: &[u8]) -> Resolver<'a, 3> {
        for (idx, channel) in channels.iter().enumerate() {
            self.0.ch[idx].pselp.write(|w| match channel {
                0 => w.pselp().analog_input0(),
                1 => w.pselp().analog_input1(),
                2 => w.pselp().analog_input2(),
                3 => w.pselp().analog_input3(),
                4 => w.pselp().analog_input4(),
                5 => w.pselp().analog_input5(),
                6 => w.pselp().analog_input6(),
                7 => w.pselp().analog_input7(),
                #[cfg(not(feature = "9160"))]
                8 => w.pselp().vdd(),
                #[cfg(any(feature = "52833", feature = "52840"))]
                13 => w.pselp().vddhdiv5(),
                _ => unreachable!(),
            });
        }
        let mut ret = Resolver {
            saadc: self,
            buffer: [0; 3],
        };
        ret.start_sample();
        ret
    }
    /// Sample channel `PIN` for the configured ADC acquisition time in differential input mode.
    /// Note that this is a blocking operation.
    pub fn read_channel<'a, PIN: Channel>(&'a mut self, _pin: &mut PIN) -> Resolver<'a, 1> {
        match PIN::CHANNEL {
            0 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input0()),
            1 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input1()),
            2 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input2()),
            3 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input3()),
            4 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input4()),
            5 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input5()),
            6 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input6()),
            7 => self.0.ch[0].pselp.write(|w| w.pselp().analog_input7()),
            #[cfg(not(feature = "9160"))]
            8 => self.0.ch[0].pselp.write(|w| w.pselp().vdd()),
            #[cfg(any(feature = "52833", feature = "52840"))]
            13 => self.0.ch[0].pselp.write(|w| w.pselp().vddhdiv5()),
            // This can never happen with the `Channel` implementations provided, as the only analog
            // pins have already been covered.
            _ => unreachable!(),
        }
        let mut ret = Resolver {
            saadc: self,
            buffer: [0; 1],
        };
        ret.start_sample();
        ret
    }
}

impl<'a, const N: usize> Resolver<'a, N> {
    #[allow(clippy::cast_possible_truncation)]
    fn start_sample(&mut self) {
        let raw_ptr = (&mut self.buffer) as *mut _;
        self.saadc
            .0
            .result
            .ptr
            .write(|w| unsafe { w.ptr().bits(raw_ptr as u32) });
        self.saadc
            .0
            .result
            .maxcnt
            .write(|w| unsafe { w.maxcnt().bits(N as u16) });
        self.saadc
            .0
            .tasks_start
            .write(|w| w.tasks_start().set_bit());
        self.saadc
            .0
            .tasks_sample
            .write(|w| w.tasks_sample().set_bit());
    }
}

impl<'a, const N: usize> Future for Resolver<'a, N> {
    type Output = [i16; N];
    fn poll(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        if self.saadc.0.events_end.read().bits() == N as u32 {
            return core::task::Poll::Ready(self.buffer);
        }
        return core::task::Poll::Pending;
    }
}

/// Used to configure the SAADC peripheral.
///
/// See the documentation of the `Default` impl for suitable default values.
pub struct SaadcConfig {
    /// Output resolution in bits.
    pub resolution: Resolution,
    /// Average 2^`oversample` input samples before transferring the result into memory.
    pub oversample: Oversample,
    /// Reference voltage of the SAADC input.
    pub reference: Reference,
    /// Gain used to control the effective input range of the SAADC.
    pub gain: Gain,
    /// Positive channel resistor control.
    pub resistor: Resistor,
    /// Acquisition time in microseconds.
    pub time: Time,
}

/// Default SAADC configuration. 0 volts reads as 0, VDD volts reads as `u16::MAX`.
/// The returned SaadcConfig is configured with the following values:
///
#[cfg_attr(feature = "52840", doc = "```")]
#[cfg_attr(not(feature = "52840"), doc = "```ignore")]
/// # use nrf_hal_common::saadc::SaadcConfig;
/// # use nrf_hal_common::pac::{saadc, SAADC};
/// # use saadc::{
/// #    ch::config::{GAIN_A as Gain, REFSEL_A as Reference, RESP_A as Resistor, TACQ_A as Time},
/// #    oversample::OVERSAMPLE_A as Oversample,
/// #    resolution::VAL_A as Resolution,
/// # };
/// # let saadc =
/// SaadcConfig {
///     resolution: Resolution::_14BIT,
///     oversample: Oversample::OVER8X,
///     reference: Reference::VDD1_4,
///     gain: Gain::GAIN1_4,
///     resistor: Resistor::BYPASS,
///     time: Time::_20US,
/// };
/// #
/// # // ensure default values haven't changed
/// # let test_saadc = SaadcConfig::default();
/// # assert_eq!(saadc.resolution, test_saadc.resolution);
/// # assert_eq!(saadc.oversample, test_saadc.oversample);
/// # assert_eq!(saadc.reference, test_saadc.reference);
/// # assert_eq!(saadc.gain, test_saadc.gain);
/// # assert_eq!(saadc.resistor, test_saadc.resistor);
/// # assert_eq!(saadc.time, test_saadc.time);
/// # ()
/// ```
impl Default for SaadcConfig {
    fn default() -> Self {
        // Note: do not forget to update the docs above if you change values here
        SaadcConfig {
            resolution: Resolution::_14BIT,
            oversample: Oversample::OVER8X,
            reference: Reference::VDD1_4,
            gain: Gain::GAIN1_4,
            resistor: Resistor::BYPASS,
            time: Time::_20US,
        }
    }
}

#[cfg(feature = "embedded-hal-02")]
impl<PIN> embedded_hal_02::adc::OneShot<Saadc, i16, PIN> for Saadc
where
    PIN: Channel,
{
    type Error = ();

    /// Sample channel `PIN` for the configured ADC acquisition time in differential input mode.
    /// Note that this is a blocking operation.
    fn read(&mut self, pin: &mut PIN) -> nb::Result<i16, Self::Error> {
        Ok(self.read_channel(pin)?)
    }
}

macro_rules! channel_mappings {
    ( $($n:expr => $pin:ident,)*) => {
        $(
            #[cfg(feature = "embedded-hal-02")]
            impl<STATE> embedded_hal_02::adc::Channel<Saadc> for crate::gpio::p0::$pin<STATE> {
                type ID = u8;

                const fn channel() -> u8 {
                    $n
                }
            }

            impl<STATE> Channel for crate::gpio::p0::$pin<STATE> {
                #[cfg(not(feature = "embedded-hal-02"))]
                const CHANNEL:u8 = $n;
            }
        )*
    };
}

#[cfg(feature = "9160")]
channel_mappings! {
    0 => P0_13,
    1 => P0_14,
    2 => P0_15,
    3 => P0_16,
    4 => P0_17,
    5 => P0_18,
    6 => P0_19,
    7 => P0_20,
}

#[cfg(not(feature = "9160"))]
channel_mappings! {
    0 => P0_02,
    1 => P0_03,
    2 => P0_04,
    3 => P0_05,
    4 => P0_28,
    5 => P0_29,
    6 => P0_30,
    7 => P0_31,
}

#[cfg(all(not(feature = "9160"), feature = "embedded-hal-02"))]
impl embedded_hal_02::adc::Channel<Saadc> for InternalVdd {
    type ID = u8;

    const fn channel() -> u8 {
        8
    }
}

#[cfg(not(feature = "9160"))]
impl Channel for InternalVdd {
    #[cfg(not(feature = "embedded-hal-02"))]
    const CHANNEL: u8 = 8;
}

#[cfg(not(feature = "9160"))]
/// Channel that doesn't sample a pin, but the internal VDD voltage.
pub struct InternalVdd;

#[cfg(all(any(feature = "52833", feature = "52840"), feature = "embedded-hal-02"))]
impl embedded_hal_02::adc::Channel<Saadc> for InternalVddHdiv5 {
    type ID = u8;

    const fn channel() -> u8 {
        13
    }
}

#[cfg(any(feature = "52833", feature = "52840"))]
impl Channel for InternalVddHdiv5 {
    #[cfg(not(feature = "embedded-hal-02"))]
    const CHANNEL: u8 = 13;
}

#[cfg(any(feature = "52833", feature = "52840"))]
/// The voltage on the VDDH pin, divided by 5.
pub struct InternalVddHdiv5;
