#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_closure)]

mod encoder;

use arrayvec::{ArrayString, ArrayVec};
use embassy_executor::Spawner;
use embassy_net::tcp::{ConnectError, TcpSocket};
use embassy_net::{Ipv4Address, StackResources};
use core::fmt::Write;
use embassy_futures::select::Either::{First, Second};
use embassy_futures::select::select;

use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_backtrace as _;
use esp_println::println;
use esp_wifi::wifi::{WifiController, WifiDevice, WifiError, WifiEvent, WifiStaDevice, WifiState};
use esp_wifi::{initialize, EspWifiInitFor};
use esp32c3_hal as hal;
use esp32c3_hal::{Cpu, Delay, Rtc};
use esp32c3_hal::gpio::{GpioPin, PullDown, RTCPinWithResistors};
use esp32c3_hal::rtc_cntl::{get_reset_reason, get_wakeup_cause, SocResetReason};
use esp32c3_hal::rtc_cntl::sleep::{RtcioWakeupSource, WakeupLevel};
use hal::clock::ClockControl;
use hal::Rng;
use hal::{embassy, IO, peripherals::Peripherals, prelude::*, timer::TimerGroup};
use static_cell::make_static;
use crate::encoder::Encoder;

const SSID: &str = "Syed";
const PASSWORD: &str = "bullseye";

#[main]
async fn main(spawner: Spawner) -> ! {

    // setup logger.
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let mut delay = Delay::new(&clocks);



    log::info!("chip up and running!");
    let reason = get_reset_reason(Cpu::ProCpu).unwrap_or(SocResetReason::ChipPowerOn);
    log::info!("reset reason: {:?}", reason);
    let wake_reason = get_wakeup_cause();
    log::info!("wake reason: {:?}", wake_reason);



    // SETUP EMBASSY
    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    embassy::init(&clocks, timer_group0.timer0);


    // SETUP ENCODER
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let mut pin = io.pins.gpio6.into_pull_down_input();
    let mut pin2 = io.pins.gpio5.into_pull_down_input();
    let mut encoder = Encoder::new(pin);
    // if !encoder.active().await  {
    //
    //
    // }
    log::info!("encoder active continuing to fetch and broadcast");

    // SETUP WIFI
    let timer = hal::systimer::SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let wifi_init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    ).unwrap();

    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&wifi_init, peripherals.WIFI, WifiStaDevice).unwrap();


    let config = embassy_net::Config::dhcpv4(Default::default());
    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*make_static!(embassy_net::Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<3>::new()),
        seed
    ));




    // SPAWN EMBASSY TASKS
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();


    // FIRMWARE LOGIC
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    let mut go_to_sleep = false;

    for i in 1..=10 {
        log::info!("waiting for link to be up {}th time...", i);
        if stack.is_link_up() {
            log::info!("link is up!");
            break;
        }

        if i >= 10 {
            go_to_sleep = true;
            break;
        }

        Timer::after(Duration::from_secs(1)).await;
    }

    if !go_to_sleep {
        log::info!("Waiting to get IP address...");
        loop {
            if let Some(config) = stack.config_v4() {
                log::info!("Got IP: {}", config.address);
                break;
            }
            Timer::after(Duration::from_millis(500)).await;
        }
    }

    loop {
        let mut val: f64 = 0.0;
        if !go_to_sleep {
            log::info!("getting val from sensor...");
            match select(encoder.get_val_avg(4), Timer::after_millis(5_000)).await {
                First(x) => {
                    val = x;
                }
                Second(_) => {
                    log::info!("did not get sensor data for 5s, sleeping...");
                    go_to_sleep = true;
                }
            }
        }


        if !go_to_sleep {
            let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);
            socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

            log::info!("connecting to server...");
            let remote_endpoint = (Ipv4Address::new(192, 168, 29, 48), 8080);

            let mut continue_with_socket = false;
            for try_count in 1..=5 {
                let conn_result = socket.connect(remote_endpoint).await;
                match conn_result {
                    Err(e) => {
                        match e {
                            ConnectError::TimedOut | ConnectError::ConnectionReset => {
                                log::info!("error connecting to server: {:?} on try {}. retrying  after 1 sec", e, try_count);
                                Timer::after_secs(1).await;
                            }
                            _ => {
                                log::info!("error connecting to server: {:?} on try {}. continuing to connect to socket anyway", e, try_count);
                                continue_with_socket = true;
                                break;
                            }


                        }
                    }

                    Ok(x) => {
                        continue_with_socket = true;
                        break;
                    }
                }


                if try_count == 5 {
                    log::info!("5 retries exhausted to connect to server.");
                    go_to_sleep = true;
                }



            }

            if continue_with_socket  {
                log::info!("connected to server!");
                let mut buf = [0; 1024];


                let mut  strr = ArrayString::<100>::new();
                write!(&mut strr, "GET /update/{}  HTTP/1.0\r\nHost: 192.168.29.182:8080\r\n\r\n", val).expect("failed to write GET request to socket.");

                if let Err(e) = socket.write(strr.as_bytes()).await {
                    log::info!("couldn't write to socket: {:?}", e);

                };

                if let Err(e) = socket.flush().await {
                    log::info!("failed to flush to socket, will retry to get val and submit again.");
                    continue

                };

                log::info!("reading socket response fully...");
                loop {
                    match socket.read(&mut buf).await {
                        Ok(n) => {
                            if n == 0 {
                                break;
                            } else {
                                continue;
                            }
                        }

                        Err(e) => {
                            println!("failed to read data from socket. error: {:?}", e);
                            break;
                        }
                    };

                }
            }
        }



        if go_to_sleep {
            // go to deep sleep

            let encoder_high = encoder.get_voltage_level().await;
            log::info!("encoder state now: {}, waking up for opposite state then", encoder_high);

            let wakeup_level = if encoder_high {
                WakeupLevel::Low
            } else {
                WakeupLevel::High
            };

            let wakeup_pins: &mut [(&mut dyn RTCPinWithResistors, WakeupLevel)] = &mut [
                (&mut pin2, wakeup_level),
            ];

            let rtcio_wakeup_source = RtcioWakeupSource::new(wakeup_pins);
            log::info!(" deep sleeping...");
            rtc.sleep_deep(&[&rtcio_wakeup_source], &mut delay);
        }

        Timer::after(Duration::from_millis(500)).await;
    }
}




#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    log::info!("start connection task");
    log::info!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if esp_wifi::wifi::get_wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            log::info!("no longer connected to wifi AP");
            Timer::after(Duration::from_millis(5000)).await
        }
        match controller.is_started() {
            Ok(started) => {
                if !started {
                    log::info!("controller not started");
                    let client_config = Configuration::Client(ClientConfiguration {
                        ssid: SSID.try_into().unwrap(),
                        password: PASSWORD.try_into().unwrap(),
                        ..Default::default()
                    });
                    controller.set_configuration(&client_config).unwrap();
                    log::info!("Starting wifi");
                    controller.start().await.unwrap();
                    log::info!("Wifi started!");
                }
            },

            Err(e) => {
                log::info!("some error occured: {:?}", e);
            }

        }
        log::info!("About to connect...");

        match controller.connect().await {
            Ok(_) => log::info!("Wifi connected!"),
            Err(e) => {
                log::info!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(1000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static embassy_net::Stack<WifiDevice<'static, WifiStaDevice>>) {
    log::info!("task#net_taask doing work on embassy stack");
    stack.run().await
}


