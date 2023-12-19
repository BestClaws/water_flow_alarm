#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_closure)]

mod encoder;

use arrayvec::{ArrayString, ArrayVec};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Ipv4Address, Stack, StackResources};
use core::fmt::Write;

use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_backtrace as _;
use esp_println::println;
use esp_wifi::wifi::{WifiController, WifiDevice, WifiEvent, WifiStaDevice, WifiState};
use esp_wifi::{initialize, EspWifiInitFor};
use esp32c3_hal as hal;
use hal::clock::ClockControl;
use hal::Rng;
use hal::{embassy, IO, peripherals::Peripherals, prelude::*, timer::TimerGroup};
use static_cell::make_static;
use crate::encoder::Encoder;

const SSID: &str = "Syed";
const PASSWORD: &str = "bullseye";

#[main]
async fn main(spawner: Spawner) -> ! {

    esp_println::logger::init_logger(log::LevelFilter::Info);

    let peripherals = Peripherals::take();

    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();



    // SETUP EMBASSY
    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    embassy::init(&clocks, timer_group0.timer0);


    // SETUP WIFI
    let timer = hal::systimer::SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    ).unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();


    let config = Config::dhcpv4(Default::default());
    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*make_static!(Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<3>::new()),
        seed
    ));


    // SETUP ENCODER
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let pin = io.pins.gpio6.into_pull_down_input();
    let mut encoder = Encoder::new(pin);

    if !encoder.active().await  {
        log::info!("encoder inactive deep sleeping...");
        // deep sleep
    }

    log::info!("encoder active continuing to fetch and broadcast");
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    log::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            log::info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    loop {
        Timer::after(Duration::from_millis(1_000)).await;

        let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(100)));

        let remote_endpoint = (Ipv4Address::new(192, 168, 29, 182), 8080);
        println!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }
        println!("connected!");
        let mut buf = [0; 1024];
        loop {
            use embedded_io_async::Write;
            log::info!("getting val from sensor...");
            let val = encoder.get_val_avg(3).await;

            let mut  strr = ArrayString::<100>::new();
            write!(&mut strr, "GET /update/{}  HTTP/1.0\r\nHost: 192.168.29.182:8080\r\n\r\n", val).expect("failed to write GET request to socket.");
            socket.write(strr.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            // let r = socket
            //     .write_all(b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n")
            //     .await;
            if let Err(e) = r {
                println!("write error: {:?}", e);
                break;
            }
            let n = match socket.read(&mut buf).await {
                Ok(n) => {
                    break;
                },
                Err(e) => {
                    println!("read error: {:?}", e);
                    break;
                }
            };
        }
        Timer::after(Duration::from_millis(300)).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    log::info!("doing work on stack");
    stack.run().await
}


