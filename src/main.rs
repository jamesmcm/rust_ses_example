mod csv_serde;
mod email;
mod s3;
#[cfg(test)]
mod tests;

// TODO: Make it work with replies - i.e. use replies to CW email, confirmation is sent as reply to
// that email - In-Reply-To, References

use crate::csv_serde::{deserialize_csv, validate_all_records};
use crate::email::{send_email, Attachment};
use crate::s3::{get_file_from_s3, write_file_to_s3};
use anyhow::Result;
use csv::Writer;
use lambda_runtime::error::HandlerError;
use log::{error, info, warn};
use mailparse::parse_mail;
use percent_encoding::percent_decode_str;
use rusoto_core::Region;
use rusoto_s3::S3Client;
use rusoto_ses::SesClient;
use serde::Deserialize;
use serde::Serialize;

static RECIPIENT: &str = "RECIPIENT NAME <recipient_email@test.com>"; // Must be verified in SES when in sandbox
static OUTPUT_BUCKET: &str = "test-file-output-bucket";
static OUTPUT_KEY: &str = "current.csv";
static FROM: &str = "SES Test <test@testses.awsapps.com>"; // Must be verified in SES, can do this with Workmail directly (i.e. here testses would be the Workmail inbox name)

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum EventEnum {
    S3Event(aws_lambda_events::event::s3::S3Event),
    CloudWatchEvent(aws_lambda_events::event::cloudwatch_events::CloudWatchEvent),
}

fn main() -> Result<()> {
    let mut builder = pretty_env_logger::formatted_timed_builder();
    builder.filter_level(log::LevelFilter::Debug);
    builder.init();
    lambda_runtime::lambda!(my_handler);

    Ok(())
}

fn my_handler(e: EventEnum, _c: lambda_runtime::Context) -> Result<(), HandlerError> {
    println!("{:?}", e);
    let s3_client = S3Client::new(Region::EuWest1);
    let ses_client = SesClient::new(Region::EuWest1);
    let mut rt = tokio::runtime::Runtime::new().unwrap();

    match e {
        // Cloudwatch event can be cron trigger
        EventEnum::CloudWatchEvent(event) => {
            info!("Cloudwatch event: {:?}", event);
            let file = get_file_from_s3(OUTPUT_BUCKET, OUTPUT_KEY, &s3_client, &mut rt).ok();
            // Try to read file
            // Send email
            if let Some(file) = file {
                send_email(
                    RECIPIENT,
                    "Please verify and update attached file",
                    "Please verify and update the attached file",
                    None,
                    Some(Attachment::new(
                        file,
                        OUTPUT_KEY.to_string(),
                        "text/csv; charset=utf8".to_string(),
                    )),
                    &ses_client,
                    &mut rt,
                )
                .ok();
            } else {
                send_email(
                    RECIPIENT,
                    "Please reply with file",
                    "Please reply with file",
                    None,
                    None,
                    &ses_client,
                    &mut rt,
                )
                .ok();
            }
        }
        // S3 trigger will be from email received in Workmail
        EventEnum::S3Event(event) => {
            let decodedkey =
                percent_decode_str(&(event.records[0].s3.object.key.as_ref()).unwrap())
                    .decode_utf8()
                    .unwrap();

            let bucket = percent_decode_str(&(event.records[0].s3.bucket.name.as_ref()).unwrap())
                .decode_utf8()
                .unwrap();

            // Rusoto above gives us Cows
            match handle_email(&bucket, &decodedkey, &s3_client, &ses_client, &mut rt) {
                Ok(_) => (),
                Err(error) => {
                    error!("Error: {:?}", error);
                    panic!("Error: {:?}", error);
                }
            }
        }
    }

    Ok(())
}

fn handle_email(
    bucket: &str,
    key: &str,
    s3_client: &S3Client,
    ses_client: &SesClient,
    rt: &mut tokio::runtime::Runtime,
) -> Result<()> {
    // Read S3 file
    let file = get_file_from_s3(bucket, key, s3_client, rt)?;

    // Parse file and read attachment
    let cap = parse_mail(&file)?;
    let attachment = cap
        .subparts
        .into_iter()
        .find(|x| x.get_content_disposition().disposition == mailparse::DispositionType::Attachment)
        .expect("No attachment");

    // TODO: Notify on missing attachment case
    let attachment_name = attachment
        .get_content_disposition()
        .params
        .get("filename")
        .expect("No filename")
        .to_string();

    let attachment = attachment.get_body()?;
    let (records, de_errors) = deserialize_csv(&attachment);

    // Validate records
    let validation_errors = validate_all_records(&records);

    // Send errors email
    if !validation_errors.is_empty() || !de_errors.is_empty() {
        let mut de_string = String::new();
        let mut validation_string = String::new();
        if !de_errors.is_empty() {
            de_string = format!(
                "Deserialization errors:\n{}",
                de_errors
                    .iter()
                    .map(|x| format!("{:?}", x))
                    .collect::<Vec<String>>()
                    .join("\n")
            );
        }
        if !validation_errors.is_empty() {
            validation_string = format!(
                "Validation errors:\n{}",
                validation_errors
                    .iter()
                    .map(|x| format!("{:?}", x))
                    .collect::<Vec<String>>()
                    .join("\n")
            );
        }

        send_email(
            RECIPIENT,
            &format!("Errors in file: {}", attachment_name),
            &format!(
                "Errors found in attached file:\n{}\n{}",
                de_string, validation_string
            ),
            None,
            Some(Attachment::new(
                attachment.as_bytes().to_vec(),
                attachment_name,
                "text/csv; charset=utf8".to_string(),
            )),
            ses_client,
            rt,
        )?;

        warn!(
            "Errors found in attached file:\n{}\n{}",
            de_string, validation_string
        );
        return Ok(());
    }

    // Write CSV to S3
    let mut wtr = Writer::from_writer(vec![]);
    for r in records {
        wtr.serialize(r)?;
    }

    let file: Vec<u8> = wtr.into_inner()?;
    write_file_to_s3(file.clone(), OUTPUT_BUCKET, OUTPUT_KEY, s3_client, rt)?;
    // Send confirmation

    send_email(
        RECIPIENT,
        "File updated successfully!",
        "File updated successfully!\nAttached for reference.",
        None,
        Some(Attachment::new(
            file,
            attachment_name,
            "text/csv; charset=utf8".to_string(),
        )),
        ses_client,
        rt,
    )?;
    Ok(())
}
