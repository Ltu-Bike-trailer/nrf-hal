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

use core::sync::atomic::{compiler_fence, Ordering::SeqCst};

use nrf52840_pac::gpiote::config;
pub use saadc::{
    ch::config::{GAIN_A as Gain, REFSEL_A as Reference, RESP_A as Resistor, TACQ_A as Time},
    oversample::OVERSAMPLE_A as Oversample,
    resolution::VAL_A as Resolution,
};

#[cfg(feature = "embedded-hal-02")]
pub trait Channel: embedded_hal_02::adc::Channel<Saadc, ID = u8> {}

#[cfg(not(feature = "embedded-hal-02"))]
pub trait Channel {
    fn channel() -> u8;
}

// Only 1 channel is allowed right now, a discussion needs to be had as to how
// multiple channels should work (See "scan mode" in the datasheet).
// Issue: https://github.com/nrf-rs/nrf-hal/issues/82

/// Interface for the SAADC peripheral.
///
/// External analog channels supported by the SAADC implement the `Channel` trait.
/// Currently, use of only one channel is allowed.
pub struct Saadc(SAADC);

pub struct SaadcTask<const CHANNELS: usize> {
    buffer: [u16; CHANNELS],
}

impl<const CHANNELS: usize> SaadcTask<CHANNELS> {
    #[inline(always)]
    fn ptr<'a>() -> &'a mut crate::pac::saadc::RegisterBlock {
        unsafe { &mut *SAADC::PTR.cast_mut() }
    }
    pub fn new(
        saadc: SAADC,
        config: SaadcConfig,
        channels: &[u8; CHANNELS],
        buffer: [u16; CHANNELS],
    ) -> Self {
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
        saadc.resolution.write(|w| w.val().variant(resolution));
        saadc
            .oversample
            .write(|w| w.oversample().variant(oversample));
        saadc.samplerate.write(|w| w.mode().task());
        for (idx, ch) in channels.iter().enumerate() {
            saadc.ch[idx].config.write(|w| {
                w.refsel().variant(reference);
                w.gain().variant(gain);
                w.tacq().variant(time);
                w.mode().se();
                w.resp().variant(resistor);
                w.resn().bypass();
                w.burst().enabled();
                w
            });

            match ch {
                0 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input0()),
                1 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input1()),
                2 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input2()),
                3 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input3()),
                4 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input4()),
                5 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input5()),
                6 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input6()),
                7 => saadc.ch[idx].pselp.write(|w| w.pselp().analog_input7()),
                #[cfg(not(feature = "9160"))]
                8 => saadc.ch[idx].pselp.write(|w| w.pselp().vdd()),
                #[cfg(any(feature = "52833", feature = "52840"))]
                13 => saadc.ch[idx].pselp.write(|w| w.pselp().vddhdiv5()),
                // This can never happen with the `Channel` implementations provided, as the only analog
                // pins have already been covered.
                _ => panic!(),
            }
            saadc.ch[idx].pseln.write(|w| w.pseln().nc());
        }

        saadc.enable.write(|w| w.enable().set_bit());
        // Calibrate
        saadc.events_calibratedone.reset();
        saadc.tasks_calibrateoffset.write(|w| unsafe { w.bits(1) });
        //while saadc.events_calibratedone.read().bits() == 0 {}
        saadc
            .inten
            .write(|w| w.end().set_bit().done().disabled().resultdone().clear_bit());
        saadc.intenset.write(|w| {
            w.end()
                .set_bit()
                .resultdone()
                .clear_bit()
                .done()
                .clear_bit()
        });
        SaadcTask { buffer }
    }

    /// Starts a new measurements cycle.
    #[inline(always)]
    pub fn start_sample(&mut self) {
        let ptr = self.buffer.as_mut_ptr();
        let saadc = Self::ptr();
        saadc.events_end.reset();
        saadc
            .result
            .ptr
            .write(|w| unsafe { w.ptr().bits(ptr as u32) });
        saadc
            .result
            .maxcnt
            .write(|w| unsafe { w.maxcnt().bits(CHANNELS as u16) });
        saadc.enable.write(|w| w.enable().set_bit());

        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        saadc.tasks_start.write(|w| w.tasks_start().set_bit());
        saadc.tasks_sample.write(|w| w.tasks_sample().set_bit());
        saadc.inten.write(|w| w.end().set_bit());
        saadc.intenset.write(|w| w.end().set_bit());
    }

    /// Prepares a sample. This is simply here to allow
    pub fn prepare_sample(&mut self) {
        let ptr = self.buffer.as_mut_ptr();
        let saadc = Self::ptr();
        saadc.events_end.reset();
        saadc
            .result
            .ptr
            .write(|w| unsafe { w.ptr().bits(ptr as u32) });
        saadc
            .result
            .maxcnt
            .write(|w| unsafe { w.maxcnt().bits(CHANNELS as u16) });
        saadc.enable.write(|w| w.enable().set_bit());

        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        saadc.tasks_start.write(|w| w.tasks_start().set_bit());
    }

    /// Reads the buffer returning the converted values.
    pub fn read_buffer<T: Default + Copy, Callback: FnMut(u16) -> T>(
        &mut self,
        mut callback: Callback,
    ) -> [T; CHANNELS] {
        let mut res = [T::default(); CHANNELS];
        for (idx, val) in self.buffer.iter().enumerate() {
            res[idx] = callback(*val);
        }
        res
    }

    /// Completes the previous measurement cycle and returns the values.
    ///
    /// This function takes a callback that allows for easy conversions.
    pub fn complete_sample<T: Default + Copy, Callback: FnMut(u16) -> T>(
        &self,
        mut callback: Callback,
    ) -> [T; CHANNELS] {
        let saadc = Self::ptr();
        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        saadc.tasks_start.write(|w| unsafe { w.bits(1) });
        saadc.tasks_sample.write(|w| unsafe { w.bits(1) });

        //while saadc.events_end.read().bits() == 0 {}
        saadc.events_end.reset();
        saadc.intenclr.write(|w| w.end().set_bit());
        //saadc.status.read().status().;

        // Second fence to prevent optimizations creating issues with the EasyDMA-modified `val`.
        //compiler_fence(SeqCst);
        let mut res = [T::default(); CHANNELS];
        for (idx, val) in self.buffer.iter().enumerate() {
            res[idx] = callback(*val);
        }

        res
    }

    pub fn sample_blocking<T: Default + Copy, Callback: FnMut(u16) -> T>(
        &mut self,
        mut callback: Callback,
    ) -> Option<[T; CHANNELS]> {
        let ptr = self.buffer.as_mut_ptr();
        let saadc = Self::ptr();
        saadc.events_end.reset();
        saadc
            .result
            .ptr
            .write(|w| unsafe { w.ptr().bits(ptr as u32) });
        saadc
            .result
            .maxcnt
            .write(|w| unsafe { w.maxcnt().bits(CHANNELS as u16) });
        saadc.enable.write(|w| w.enable().set_bit());

        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        saadc.tasks_start.write(|w| w.tasks_start().set_bit());
        saadc.tasks_sample.write(|w| w.tasks_sample().set_bit());
        saadc.inten.write(|w| w.end().clear_bit());
        saadc.intenset.write(|w| w.end().clear_bit());

        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        saadc.tasks_start.write(|w| unsafe { w.bits(1) });
        saadc.tasks_sample.write(|w| unsafe { w.bits(1) });
        let mut count = 0;
        while saadc.events_end.read().bits() == 0 {
            count += 1;
            if count > 10_000 {
                return None;
            }
        }
        saadc.events_end.reset();

        // Second fence to prevent optimizations creating issues with the EasyDMA-modified `val`.
        compiler_fence(SeqCst);
        let mut res = [T::default(); CHANNELS];
        for (idx, val) in self.buffer.iter().enumerate() {
            res[idx] = callback(*val);
        }

        Some(res)
    }
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

    /// Sample channel `PIN` for the configured ADC acquisition time in differential input mode.
    /// Note that this is a blocking operation.
    pub fn read_channel<PIN: Channel>(&mut self, _pin: &mut PIN) -> Result<i16, ()> {
        match PIN::channel() {
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
            _ => return Err(()),
        }

        let mut val: i16 = 0;
        self.0
            .result
            .ptr
            .write(|w| unsafe { w.ptr().bits(((&mut val) as *mut _) as u32) });
        self.0
            .result
            .maxcnt
            .write(|w| unsafe { w.maxcnt().bits(1) });

        // Conservative compiler fence to prevent starting the ADC before the
        // pointer and maxcount have been set.
        compiler_fence(SeqCst);

        self.0.tasks_start.write(|w| unsafe { w.bits(1) });
        self.0.tasks_sample.write(|w| unsafe { w.bits(1) });

        while self.0.events_end.read().bits() == 0 {}
        self.0.events_end.reset();

        // Will only occur if more than one channel has been enabled.
        if self.0.result.amount.read().bits() != 1 {
            return Err(());
        }

        // Second fence to prevent optimizations creating issues with the EasyDMA-modified `val`.
        compiler_fence(SeqCst);

        Ok(val)
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

                fn channel() -> u8 {
                    $n
                }
            }

            impl<STATE> Channel for crate::gpio::p0::$pin<STATE> {
                #[cfg(not(feature = "embedded-hal-02"))]
                fn channel() -> u8 {
                    $n
                }
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

    fn channel() -> u8 {
        8
    }
}

#[cfg(not(feature = "9160"))]
impl Channel for InternalVdd {
    #[cfg(not(feature = "embedded-hal-02"))]
    fn channel() -> u8 {
        8
    }
}

#[cfg(not(feature = "9160"))]
/// Channel that doesn't sample a pin, but the internal VDD voltage.
pub struct InternalVdd;

#[cfg(all(any(feature = "52833", feature = "52840"), feature = "embedded-hal-02"))]
impl embedded_hal_02::adc::Channel<Saadc> for InternalVddHdiv5 {
    type ID = u8;

    fn channel() -> u8 {
        13
    }
}

#[cfg(any(feature = "52833", feature = "52840"))]
impl Channel for InternalVddHdiv5 {
    #[cfg(not(feature = "embedded-hal-02"))]
    fn channel() -> u8 {
        13
    }
}

#[cfg(any(feature = "52833", feature = "52840"))]
/// The voltage on the VDDH pin, divided by 5.
pub struct InternalVddHdiv5;
