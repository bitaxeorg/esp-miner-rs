//! Statistics history: samples of the miner's measurements, for the charts.
//!
//! Each value is kept as a `u16` with a per-column scale, `u16::MAX` standing for no value.

use crate::json::JsonWriter;

/// The columns, in the order of the statistics API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Hashrate,
    Hashrate1m,
    Hashrate10m,
    Hashrate1h,
    ErrorPercentage,
    AsicTemp,
    AsicTemp2,
    VrTemp,
    AsicVoltage,
    Voltage,
    Power,
    Current,
    FanSpeed,
    FanRpm,
    Fan2Rpm,
    WifiRssi,
    FreeHeap,
    ResponseTimeMin,
    ResponseTimeMax,
}

pub const COLUMNS: usize = 19;

impl Column {
    pub const ALL: [Column; COLUMNS] = [
        Column::Hashrate,
        Column::Hashrate1m,
        Column::Hashrate10m,
        Column::Hashrate1h,
        Column::ErrorPercentage,
        Column::AsicTemp,
        Column::AsicTemp2,
        Column::VrTemp,
        Column::AsicVoltage,
        Column::Voltage,
        Column::Power,
        Column::Current,
        Column::FanSpeed,
        Column::FanRpm,
        Column::Fan2Rpm,
        Column::WifiRssi,
        Column::FreeHeap,
        Column::ResponseTimeMin,
        Column::ResponseTimeMax,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Column::Hashrate => "hashrate",
            Column::Hashrate1m => "hashrate_1m",
            Column::Hashrate10m => "hashrate_10m",
            Column::Hashrate1h => "hashrate_1h",
            Column::ErrorPercentage => "errorPercentage",
            Column::AsicTemp => "asicTemp",
            Column::AsicTemp2 => "asicTemp2",
            Column::VrTemp => "vrTemp",
            Column::AsicVoltage => "asicVoltage",
            Column::Voltage => "voltage",
            Column::Power => "power",
            Column::Current => "current",
            Column::FanSpeed => "fanSpeed",
            Column::FanRpm => "fanRpm",
            Column::Fan2Rpm => "fan2Rpm",
            Column::WifiRssi => "wifiRssi",
            Column::FreeHeap => "freeHeap",
            Column::ResponseTimeMin => "responseTimeMin",
            Column::ResponseTimeMax => "responseTimeMax",
        }
    }

    pub fn from_name(name: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.name() == name)
    }

    /// Stored value per unit, and whether the value is signed. Units: GH/s, %, °C, mV, mA, W,
    /// RPM, dBm, bytes (stored / 4), ms.
    fn scale(self) -> (f32, bool) {
        match self {
            Column::Hashrate | Column::Hashrate1m | Column::Hashrate10m | Column::Hashrate1h => {
                (1.0, false)
            }
            Column::ErrorPercentage => (100.0, false),
            Column::AsicTemp | Column::AsicTemp2 | Column::VrTemp => (10.0, true),
            Column::AsicVoltage | Column::Voltage | Column::Current => (1.0, false),
            Column::Power | Column::FanSpeed => (10.0, false),
            Column::FanRpm | Column::Fan2Rpm => (1.0, false),
            Column::WifiRssi => (1.0, true),
            Column::FreeHeap => (0.25, false),
            Column::ResponseTimeMin | Column::ResponseTimeMax => (10.0, false),
        }
    }

    /// Whether the API gives this column as an integer.
    fn integer(self) -> bool {
        matches!(
            self,
            Column::AsicVoltage
                | Column::FanRpm
                | Column::Fan2Rpm
                | Column::WifiRssi
                | Column::FreeHeap
        )
    }

    fn encode(self, value: Option<f32>) -> u16 {
        let Some(v) = value.filter(|v| v.is_finite()) else {
            return u16::MAX;
        };
        let (scale, signed) = self.scale();
        let scaled = round(v * scale);
        if signed {
            scaled.clamp(i16::MIN as f32, (i16::MAX - 1) as f32) as i16 as u16
        } else {
            scaled.clamp(0.0, (u16::MAX - 1) as f32) as u16
        }
    }

    fn decode(self, raw: u16) -> Option<f32> {
        let (scale, signed) = self.scale();
        if signed {
            (raw != i16::MAX as u16 && raw != u16::MAX).then(|| raw as i16 as f32 / scale)
        } else {
            (raw != u16::MAX).then(|| raw as f32 / scale)
        }
    }
}

/// `value` rounded to the nearest integer, halves away from zero.
fn round(value: f32) -> f32 {
    if value >= 0.0 {
        (value + 0.5) as i64 as f32
    } else {
        (value - 0.5) as i64 as f32
    }
}

/// One sample: the values of every column, `None` when unknown.
pub type Values = [Option<f32>; COLUMNS];

/// The last `N` samples.
pub struct StatsRing<const N: usize> {
    samples: [[u16; COLUMNS]; N],
    timestamps: [u32; N],
    /// Index of the oldest sample.
    start: usize,
    len: usize,
}

impl<const N: usize> Default for StatsRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> StatsRing<N> {
    pub const fn new() -> Self {
        Self {
            samples: [[u16::MAX; COLUMNS]; N],
            timestamps: [0; N],
            start: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
    }

    /// Adds a sample taken at `timestamp_ms`, dropping the oldest when full.
    pub fn push(&mut self, timestamp_ms: u32, values: &Values) {
        let index = (self.start + self.len) % N;
        for (raw, (column, value)) in self.samples[index]
            .iter_mut()
            .zip(Column::ALL.iter().zip(values))
        {
            *raw = column.encode(*value);
        }
        self.timestamps[index] = timestamp_ms;
        if self.len < N {
            self.len += 1;
        } else {
            self.start = (self.start + 1) % N;
        }
    }

    /// Timestamp and value of `column` in each sample, oldest first.
    pub fn series(&self, column: Column) -> impl Iterator<Item = (u32, Option<f32>)> + '_ {
        let c = column as usize;
        (0..self.len).map(move |i| {
            let index = (self.start + i) % N;
            (
                self.timestamps[index],
                column.decode(self.samples[index][c]),
            )
        })
    }

    /// Writes the statistics API object: the selected columns (all when none), the time of the
    /// request, and the samples.
    pub fn write_json<W: core::fmt::Write>(
        &self,
        json: &mut JsonWriter<W>,
        columns: &[Column],
        now_ms: u32,
    ) {
        let selected = |c: &Column| columns.is_empty() || columns.contains(c);
        json.begin_object().field_num("currentTimestamp", now_ms);
        json.key("labels").begin_array();
        for c in Column::ALL.iter().filter(|c| selected(c)) {
            json.str(c.name());
        }
        json.str("timestamp").end_array();
        json.key("statistics").begin_array();
        for i in 0..self.len {
            let index = (self.start + i) % N;
            json.begin_array();
            for c in Column::ALL.iter().filter(|c| selected(c)) {
                match c.decode(self.samples[index][*c as usize]) {
                    None => json.null(),
                    Some(v) if c.integer() => json.num(v as i64),
                    Some(v) => json.fixed(v.into(), 2),
                };
            }
            json.num(self.timestamps[index]).end_array();
        }
        json.end_array().end_object();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::String;

    use super::*;

    fn values(hashrate: f32, temp: Option<f32>, rssi: f32) -> Values {
        let mut v = [None; COLUMNS];
        v[Column::Hashrate as usize] = Some(hashrate);
        v[Column::AsicTemp as usize] = temp;
        v[Column::WifiRssi as usize] = Some(rssi);
        v[Column::FreeHeap as usize] = Some(51_200.0);
        v
    }

    #[test]
    fn values_round_trip_with_their_scale() {
        let mut ring = StatsRing::<4>::new();
        ring.push(1000, &values(1234.4, Some(61.25), -57.0));
        let (t, v) = ring.series(Column::Hashrate).next().unwrap();
        assert_eq!((t, v), (1000, Some(1234.0)));
        assert_eq!(ring.series(Column::AsicTemp).next().unwrap().1, Some(61.3));
        assert_eq!(ring.series(Column::WifiRssi).next().unwrap().1, Some(-57.0));
        assert_eq!(
            ring.series(Column::FreeHeap).next().unwrap().1,
            Some(51_200.0)
        );
        assert_eq!(ring.series(Column::VrTemp).next().unwrap().1, None);
    }

    #[test]
    fn the_oldest_sample_is_dropped_when_full() {
        let mut ring = StatsRing::<3>::new();
        for t in 1..=5 {
            ring.push(t, &values(t as f32, None, 0.0));
        }
        let hashrates: std::vec::Vec<_> = ring
            .series(Column::Hashrate)
            .map(|(t, v)| (t, v.unwrap()))
            .collect();
        assert_eq!(hashrates, [(3, 3.0), (4, 4.0), (5, 5.0)]);
    }

    #[test]
    fn json_of_selected_columns() {
        let mut ring = StatsRing::<4>::new();
        ring.push(500, &values(100.0, None, -60.0));
        let mut json = JsonWriter::new(String::new());
        ring.write_json(
            &mut json,
            &[Column::WifiRssi, Column::Hashrate, Column::AsicTemp],
            900,
        );
        assert_eq!(
            json.finish().unwrap(),
            r#"{"currentTimestamp":900,"labels":["hashrate","asicTemp","wifiRssi","timestamp"],"statistics":[[100.00,null,-60,500]]}"#
        );
        assert_eq!(Column::from_name("fan2Rpm"), Some(Column::Fan2Rpm));
        assert_eq!(Column::from_name("bogus"), None);
    }
}
