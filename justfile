set export

SSID := "my_ssid"
PASSWORD := "my_password"

build b:
  cargo build --release --features=bitaxe-$b

run b:
  cargo run --release --features=bitaxe-$b
