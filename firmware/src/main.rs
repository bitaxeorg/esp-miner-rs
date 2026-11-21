#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![recursion_limit = "512"]
// A devkit compiles the board code without running it.
#![cfg_attr(feature = "devkit", allow(dead_code, unused_imports))]

extern crate alloc;

mod asic;
mod board;
mod clock;
mod cooling;
#[cfg(any(feature = "display-oled", feature = "display-st7789"))]
mod display;
mod flash;
mod net;
mod ota;
mod pool;
mod power;
mod settings;
mod stats;
mod status;
mod vcore;
mod web;
mod weblog;

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
use esp_radio::wifi::{Config, ControllerConfig, Interface, WifiController};
use picoserve::AppBuilder;
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

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // The heap: the RAM the second-stage bootloader used, free once the application runs, and
    // part of the data RAM, whose rest is the main stack. The stratum client alone takes 26 KiB
    // of stack when it parses a message.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 40 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    flash::init(peripherals.FLASH).await;
    settings::load().await;
    ota::confirm().await;
    spawner.spawn(web::restart_task().unwrap());
    spawner.spawn(stats::stats_task().unwrap());

    // Power on I2C0 (SDA GPIO47, SCL GPIO48, or GPIO44 and GPIO43 on the boards with the colour
    // display): the core regulator (see `vcore`) and the fan controller, which also reads the
    // ASIC temperature (see `cooling`).
    // A devkit has none of these, nor the ASIC chain.
    #[cfg(not(feature = "devkit"))]
    static I2C_BUS: StaticCell<Mutex<NoopRawMutex, I2c<'_, esp_hal::Async>>> = StaticCell::new();
    #[cfg(not(feature = "devkit"))]
    let i2c_bus = {
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
        I2C_BUS.init(Mutex::new(i2c.into_async()))
    };
    #[cfg(not(feature = "devkit"))]
    {
        // DS4432 (core voltage) and INA260 (input power). The regulator enable (PWR_EN, GPIO10) is
        // active low: off until the power task has set the core voltage. The core voltage is read
        // on ADC1 (GPIO2), calibrated.
        #[cfg(feature = "vcore-dac")]
        let vcore = {
            use esp_hal::analog::adc::{Adc, AdcCalCurve, AdcConfig, Attenuation};
            let mut adc_config = AdcConfig::new();
            let vcore_pin = adc_config
                .enable_pin_with_cal::<_, AdcCalCurve<_>>(peripherals.GPIO2, Attenuation::_11dB);
            vcore::Vcore::new(
                ds4432::AsyncDS4432::with_rfs(
                    I2cDevice::new(i2c_bus),
                    Some(board::DS4432_RFS0_OHM),
                    None,
                )
                .unwrap(),
                ina260::AsyncINA260::new(I2cDevice::new(i2c_bus))
                    .await
                    .unwrap(),
                Output::new(peripherals.GPIO10, Level::High, OutputConfig::default()),
                Adc::new(peripherals.ADC1, adc_config).into_async(),
                vcore_pin,
            )
        };
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
        #[cfg(feature = "fan-emc2302")]
        let cooling = cooling::Cooling::new(
            emc230x::Emc230x::new(I2cDevice::new(i2c_bus), board::EMC2302_ADDRESS)
                .await
                .unwrap(),
            board::TMP1075_ADDRESSES
                .map(|address| tmp1075::Tmp1075::with_address(address, I2cDevice::new(i2c_bus))),
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
    }

    // WiFi: the station of the settings, and the open setup access point until the station has
    // an address
    let (configured, hostname) = settings::with(|s| (!s.ssid.is_empty(), s.hostname.clone()));
    let station_interface = Interface::station();
    let ap_interface = Interface::access_point();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(Config::AccessPointStation(
            net::station_config(),
            net::ap_config(),
        )),
    )
    .unwrap();

    let mut dhcp = embassy_net::DhcpConfig::default();
    dhcp.hostname = heapless::String::try_from(hostname.as_str()).ok();
    let config = embassy_net::Config::dhcpv4(dhcp);

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        station_interface,
        config,
        mk_static!(StackResources<8>, StackResources::<8>::new()),
        seed,
    );
    let ap_config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(net::AP_ADDRESS, net::AP_PREFIX),
        gateway: Some(net::AP_ADDRESS),
        dns_servers: Default::default(),
    });
    let (ap_stack, ap_runner) = embassy_net::new(
        ap_interface,
        ap_config,
        mk_static!(StackResources<6>, StackResources::<6>::new()),
        seed.rotate_left(32),
    );

    spawner.spawn(net::connection_task(controller, configured).unwrap());
    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(net_task(ap_runner).unwrap());
    spawner.spawn(net::address_task(stack).unwrap());
    spawner.spawn(net::dhcp_task(ap_stack).unwrap());
    spawner.spawn(net::captive_dns_task(ap_stack).unwrap());
    spawner.spawn(net::mdns_task(stack, rng).unwrap());

    // web interface, on the station and on the access point
    let app = mk_static!(picoserve::AppRouter<web::App>, web::App.build_app());
    for id in 0..web::STATION_TASKS {
        spawner.spawn(web::station_task(id, stack, app).unwrap());
    }
    for id in 0..web::AP_TASKS {
        spawner.spawn(web::ap_task(id, ap_stack, app).unwrap());
    }

    // The BOOT button (GPIO0, active low) moves the status screens on.
    #[cfg(any(feature = "display-oled", feature = "display-st7789"))]
    let button = esp_hal::gpio::Input::new(
        peripherals.GPIO0,
        esp_hal::gpio::InputConfig::default().with_pull(esp_hal::gpio::Pull::Up),
    );

    // Status screens on the OLED, on the shared I2C bus
    #[cfg(feature = "display-oled")]
    spawner.spawn(display::display_task(I2cDevice::new(i2c_bus), stack, button).unwrap());

    // Status screens on the ST7789: 8-bit i80 bus on LCD_CAM (data GPIO39 to GPIO42 and GPIO45 to
    // GPIO48, WR GPIO8, DC GPIO7, CS GPIO6, 4 MHz), RD GPIO9 held high, reset GPIO5 (active low),
    // panel power GPIO15 and backlight GPIO38 (active high).
    #[cfg(feature = "display-st7789")]
    {
        use esp_hal::lcd_cam::{
            lcd::i8080::{self, I8080},
            LcdCam,
        };
        let lcd_cam = LcdCam::new(peripherals.LCD_CAM).into_async();
        let i8080 = I8080::new(
            lcd_cam.lcd,
            peripherals.DMA_CH0,
            i8080::Config::default().with_frequency(Rate::from_mhz(4)),
        )
        .unwrap()
        .with_cs(peripherals.GPIO6)
        .with_dc(peripherals.GPIO7)
        .with_wrx(peripherals.GPIO8)
        .with_data0(peripherals.GPIO39)
        .with_data1(peripherals.GPIO40)
        .with_data2(peripherals.GPIO41)
        .with_data3(peripherals.GPIO42)
        .with_data4(peripherals.GPIO45)
        .with_data5(peripherals.GPIO46)
        .with_data6(peripherals.GPIO47)
        .with_data7(peripherals.GPIO48);
        let lcd = display::Lcd::new(
            i8080,
            esp_hal::dma_tx_buffer!(display::ROW_BYTES).unwrap(),
            Output::new(peripherals.GPIO5, Level::High, OutputConfig::default()),
            Output::new(peripherals.GPIO38, Level::Low, OutputConfig::default()),
            Output::new(peripherals.GPIO9, Level::High, OutputConfig::default()),
            Output::new(peripherals.GPIO15, Level::High, OutputConfig::default()),
        );
        spawner.spawn(display::display_task(lcd, stack, button).unwrap());
    }

    // Pool session
    spawner.spawn(
        pool::pool_task(
            stack,
            mk_static!([u8; 1536], [0; 1536]),
            mk_static!([u8; 1536], [0; 1536]),
        )
        .unwrap(),
    );

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
