set export

# Optional default WiFi credentials; empty, the miner starts its setup access point.
WIFI_SSID := ""
WIFI_PASSWORD := ""

build b:
  cd firmware && cargo build --release --features=bitaxe-$b

run b:
  cd firmware && cargo run --release --features=bitaxe-$b

test:
  cargo test

# regenerate the display fonts and images from their LVGL sources
display-assets sources unscii:
  cargo run -p display-assets -- $sources $unscii
