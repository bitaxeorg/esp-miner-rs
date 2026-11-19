#![no_std]
#![no_main]

mod asic;
mod board;
mod cooling;
mod io06;
mod pool;
mod power;
mod vcore;

use defmt::{debug, error, info};
use defmt_rtt as _;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_net::{Runner, StackResources};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    i2c::master::I2c,
    rng::Rng,
    time::Rate,
    timer::timg::TimerGroup,
    uart::{self, Uart},
};
use esp_radio::wifi::{
    sta::StationConfig, AuthenticationMethodConfig, Config, ControllerConfig, Interface,
    WifiController,
};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // Power on I2C0 (SDA GPIO47, SCL GPIO48, or GPIO44 and GPIO43 on the boards with the colour
    // display): the core regulator (see `vcore`) and the fan controller, which also reads the
    // ASIC temperature (see `cooling`).
    static I2C_BUS: StaticCell<Mutex<NoopRawMutex, I2c<'_, esp_hal::Async>>> = StaticCell::new();
    let i2c = I2c::new(
        peripherals.I2C0,
        esp_hal::i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap();
    #[cfg(not(feature = "pins-color"))]
    let i2c = i2c
        .with_sda(peripherals.GPIO47)
        .with_scl(peripherals.GPIO48);
    #[cfg(feature = "pins-color")]
    let i2c = i2c
        .with_sda(peripherals.GPIO44)
        .with_scl(peripherals.GPIO43);
    let i2c = i2c.into_async();
    let i2c_bus = I2C_BUS.init(Mutex::new(i2c));
    // DS4432 (core voltage) and INA260 (input power). The regulator enable (PWR_EN, GPIO10) is
    // active low: off until the power task has set the core voltage.
    #[cfg(feature = "vcore-dac")]
    let vcore = vcore::Vcore::new(
        ds4432::AsyncDS4432::with_rfs(I2cDevice::new(i2c_bus), Some(board::DS4432_RFS0_OHM), None)
            .unwrap(),
        ina260::AsyncINA260::new(I2cDevice::new(i2c_bus))
            .await
            .unwrap(),
        Output::new(peripherals.GPIO10, Level::High, OutputConfig::default()),
    );
    // TPS546 over PMBus, off until the power task has configured it, and on some boards an ASIC
    // power enable (GPIO10) active high, off until then too.
    #[cfg(feature = "vcore-tps546")]
    let vcore = vcore::Vcore::new(
        I2cDevice::new(i2c_bus),
        #[cfg(feature = "asic-enable")]
        Some(Output::new(
            peripherals.GPIO10,
            Level::Low,
            OutputConfig::default(),
        )),
        #[cfg(not(feature = "asic-enable"))]
        None,
    );
    #[cfg(feature = "fan-emc2101")]
    let cooling = cooling::Cooling::new(
        emc2101::AsyncEMC2101::new(I2cDevice::new(i2c_bus))
            .await
            .unwrap(),
    );
    #[cfg(feature = "fan-emc2103")]
    let cooling = cooling::Cooling::new(
        emc2103::AsyncEMC2103::new(I2cDevice::new(i2c_bus))
            .await
            .unwrap(),
    );
    spawner.spawn(power::power_task(vcore, cooling).unwrap());

    // ASIC chain: UART1 (TX GPIO17, RX GPIO18), NRST on GPIO1 (active low).
    let uart_config = uart::Config::default().with_baudrate(115_200);
    let uart = Uart::new(peripherals.UART1, uart_config)
        .unwrap()
        .with_tx(peripherals.GPIO17)
        .with_rx(peripherals.GPIO18)
        .into_async();
    let reset = Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default());
    spawner.spawn(asic::asic_task(asic::AsicUart::new(uart, uart_config), reset).unwrap());

    // WiFi and network stack
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                WIFI_PASSWORD.try_into().unwrap(),
            )),
    );
    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection_task(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    // Pool session
    spawner.spawn(
        pool::pool_task(
            stack,
            mk_static!([u8; 1536], [0; 1536]),
            mk_static!([u8; 1536], [0; 1536]),
        )
        .unwrap(),
    );

    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        info!("Got IP: {}", config.address);
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    debug!("start connection task");
    loop {
        debug!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => {
                debug!("Wifi connected!");
                // wait until we're no longer connected
                controller.wait_for_disconnect_async().await.ok();
                debug!("Wifi disconnected");
            }
            Err(e) => {
                error!("Failed to connect to wifi: {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(5000)).await
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
