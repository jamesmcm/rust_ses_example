use anyhow::{anyhow, Result};
use chrono::naive::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Entry {
    #[serde(alias = "ID")]
    pub id: u32,
    #[serde(deserialize_with = "de_datetime", serialize_with = "se_datetime")]
    pub start_date: NaiveDateTime,
    #[serde(deserialize_with = "de_datetime", serialize_with = "se_datetime")]
    pub end_date: NaiveDateTime,
}

fn de_datetime<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
        Ok(x) => Ok(x),
        Err(x) => Err(serde::de::Error::custom(x)),
    }
}

fn se_datetime<S>(dt: &NaiveDateTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let t = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    serializer.collect_str(&t)
}

pub fn deserialize_csv(attachment: &str) -> (Vec<Entry>, Vec<csv::Error>) {
    let mut rdr = csv::Reader::from_reader(Cursor::new(attachment.trim()));
    let mut records: Vec<Entry> = Vec::with_capacity(16);
    let mut de_errors: Vec<csv::Error> = Vec::new();
    for result in rdr.deserialize() {
        match result {
            Ok(record) => {
                records.push(record);
            }
            Err(error) => de_errors.push(error),
        }
    }

    (records, de_errors)
}

fn validate_record(r: &Entry) -> Result<()> {
    if r.start_date > r.end_date {
        Err(anyhow!(
            "Start date after end date for entry: {}, {}, {}",
            r.id,
            r.start_date,
            r.end_date
        ))
    } else {
        Ok(())
    }
}

pub fn validate_all_records(records: &[Entry]) -> Vec<anyhow::Error> {
    records
        .iter()
        .map(|x| validate_record(x))
        .filter_map(|x| x.err())
        .collect()
}
