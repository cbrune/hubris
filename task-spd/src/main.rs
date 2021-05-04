//! SPD proxy task
//!
//! This is (or will be) a I2C proxy for SPD data -- but at the moment it just
//! proxies sensor data.
//!

#![no_std]
#![no_main]

#[cfg(feature = "h7b3")]
use stm32h7::stm32h7b3 as device;

#[cfg(feature = "h743")]
use stm32h7::stm32h743 as device;

use drv_i2c_api::*;
use drv_i2c_api::{Controller, Port};
use drv_stm32h7_gpio_api::*;
use drv_stm32h7_i2c::*;
use drv_stm32h7_rcc_api::{Peripheral, Rcc};
use ringbuf::*;
use userlib::*;

#[cfg(not(feature = "standalone"))]
const RCC: Task = Task::rcc_driver;

#[cfg(feature = "standalone")]
const RCC: Task = Task::anonymous;

#[cfg(not(feature = "standalone"))]
const GPIO: Task = Task::gpio_driver;

#[cfg(feature = "standalone")]
const GPIO: Task = Task::anonymous;

#[cfg(not(feature = "standalone"))]
const I2C: Task = Task::i2c_driver;

#[cfg(feature = "standalone")]
const I2C: Task = Task::anonymous;

fn configure_pin(pin: &I2cPin) {
    let gpio_driver =
        TaskId::for_index_and_gen(GPIO as usize, Generation::default());
    let gpio_driver = Gpio::from(gpio_driver);

    gpio_driver
        .configure(
            pin.gpio_port,
            pin.mask,
            Mode::Alternate,
            OutputType::OpenDrain,
            Speed::High,
            Pull::None,
            pin.function,
        )
        .unwrap();
}

const ADT7420_ADDRESS: u8 = 0x48;

const ADT7420_REG_TEMPMSB: u8 = 0;
const ADT7420_REG_ID: u8 = 0xb;

ringbuf!(u8, 16, 0);

#[export_name = "main"]
fn main() -> ! {
    cfg_if::cfg_if! {
        if #[cfg(target_board = "gemini-bu-1")] {
            let controller = I2cController {
                controller: Controller::I2C2,
                peripheral: Peripheral::I2c2,
                registers: unsafe { &*device::I2C2::ptr() },
                notification: (1 << (2 - 1)),
            };

            let pin = I2cPin {
                controller: Controller::I2C2,
                port: Port::F,
                gpio_port: drv_stm32h7_gpio_api::Port::F,
                function: Alternate::AF4,
                mask: (1 << 0) | (1 << 1),
            };
        } else {
            cfg_if::cfg_if! {
                if #[cfg(feature = "standalone")] {
                    let controller = I2cController {
                        controller: Controller::I2C2,
                        peripheral: Peripheral::I2c2,
                        registers: unsafe { &*device::I2C2::ptr() },
                        notification: (1 << (2 - 1)),
                    };

                    let pin = I2cPin {
                        controller: Controller::I2C2,
                        port: Port::F,
                        gpio_port: drv_stm32h7_gpio_api::Port::F,
                        function: Alternate::AF4,
                        mask: (1 << 0) | (1 << 1),
                    };
                } else {
                    compile_error!("I2C target unsupported for this board");
                }
            }
        }
    }

    // Enable the controller
    let rcc_driver = Rcc::from(TaskId::for_index_and_gen(
        RCC as usize,
        Generation::default(),
    ));

    controller.enable(&rcc_driver);

    // Configure our pins
    configure_pin(&pin);

    let devices = [
        I2cDevice::new(
            TaskId::for_index_and_gen(I2C as usize, Generation::default()),
            Controller::I2C4,
            Port::F,
            Some((Mux::M1, Segment::S1)),
            ADT7420_ADDRESS,
        ),
        I2cDevice::new(
            TaskId::for_index_and_gen(I2C as usize, Generation::default()),
            Controller::I2C4,
            Port::F,
            Some((Mux::M1, Segment::S4)),
            ADT7420_ADDRESS,
        ),
    ];

    ringbuf_entry!(0);

    let mut response = |addr, register, buf: &mut [u8]| -> Option<usize> {
        ringbuf_entry!(addr);
        let device: &I2cDevice = if addr == ADT7420_ADDRESS - 1 {
            &devices[0]
        } else if addr == ADT7420_ADDRESS + 1 {
            &devices[1]
        } else {
            sys_log!("bogus addr {:x}", addr);
            return None;
        };

        match register {
            Some(val) if val == ADT7420_REG_TEMPMSB => {
                match device.read_reg::<u8, [u8; 2]>(0 as u8) {
                    Ok(rval) => {
                        buf[0] = rval[0];
                        buf[1] = rval[1];

                        sys_log!("returning {:x} {:x}", rval[0], rval[1]);
                        Some(2)
                    }

                    Err(err) => {
                        sys_log!("failed to read temp: {:?}", err);
                        buf[0] = 0xff;
                        Some(1)
                    }
                }
            }

            Some(val) if val == ADT7420_REG_ID => {
                match device.read_reg::<u8, u8>(val) {
                    Ok(rval) => {
                        buf[0] = rval;
                        Some(1)
                    }

                    Err(err) => {
                        sys_log!("failed to read reg {:x}: {:?}", val, err);
                        buf[0] = 0xff;
                        Some(1)
                    }
                }
            }

            _ => {
                buf[0] = 0xfe;
                None
            }
        }
    };

    let ctrl = I2cControl {
        enable: |notification| {
            sys_irq_control(notification, true);
        },
        wfi: |notification| {
            let _ = sys_recv_closed(&mut [], notification, TaskId::KERNEL);
        },
    };

    controller.operate_as_target(
        ADT7420_ADDRESS - 1,
        Some(ADT7420_ADDRESS + 1),
        &ctrl,
        &mut response,
    );
}