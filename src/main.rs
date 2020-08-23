#[cfg(test)]
mod tests;

// TODO: Make it work with replies - i.e. use replies to CW email, confirmation is sent as reply to
// that email - In-Reply-To, References

use anyhow::{anyhow, Result};
use chrono::naive::NaiveDateTime;
use csv::Writer;
use lambda_runtime::error::HandlerError;
use lettre::message::header::{Charset, DispositionParam, DispositionType};
use lettre::message::{header, Message, MultiPart, SinglePart};
use log::{debug, error, info, warn};
use mailparse::parse_mail;
use percent_encoding::percent_decode_str;
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, PutObjectRequest, S3Client, S3};
use rusoto_ses::Ses;
use rusoto_ses::SesClient;
use serde::Deserialize;
use serde::Serialize;
use std::io::Cursor;
use std::io::Read;

static RECIPIENT: &str = "James McMurray <jamesmcm03@gmail.com>";
static OUTPUT_BUCKET: &str = "test-file-output-bucket";
static OUTPUT_KEY: &str = "current.csv";
static FROM: &str = "SES Test <test@testjamesmcm.awsapps.com>";

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Entry {
    #[serde(alias = "ID")]
    id: u32,
    #[serde(deserialize_with = "de_datetime", serialize_with = "se_datetime")]
    start_date: NaiveDateTime,
    #[serde(deserialize_with = "de_datetime", serialize_with = "se_datetime")]
    end_date: NaiveDateTime,
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
        EventEnum::CloudWatchEvent(event) => {
            info!("Cloudwatch event: {:?}", event);
            let file = get_file_from_s3(OUTPUT_BUCKET, OUTPUT_KEY, &s3_client, &mut rt).ok();
            // Try to read file
            //
            // Send email
            if file.is_some() {
                send_email(
                    RECIPIENT,
                    "Please verify and update attached file",
                    "Please verify and update the attached file",
                    None,
                    Some(Attachment {
                        attachment: file.unwrap(),
                        name: OUTPUT_KEY.to_string(),
                        mime: "text/csv; charset=utf8".to_string(),
                    }),
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

    let keypath = std::path::PathBuf::from(key);
    let file_name = keypath.file_name().unwrap().to_str().unwrap().to_string();
    // Parse file and read attachment
    let cap = parse_mail(&file)?;
    let attachment = cap
        .subparts
        .into_iter()
        .filter(|x| {
            x.get_content_disposition().disposition == mailparse::DispositionType::Attachment
        })
        .next()
        .expect("No attachment");

    // TODO: Notify on missing attachment case
    let attachment_name = attachment
        .get_content_disposition()
        .params
        .get("filename")
        .expect("No filename")
        .to_string();

    let attachment = attachment.get_body()?;

    let mut rdr = csv::Reader::from_reader(Cursor::new(attachment.trim()));
    let mut records: Vec<Entry> = Vec::with_capacity(16);

    let mut de_errors: Vec<csv::Error> = Vec::new();
    for result in rdr.deserialize() {
        // TODO: Handle errors
        match result {
            Ok(record) => {
                records.push(record);
            }
            Err(error) => de_errors.push(error),
        }
    }

    // Validate records
    let validation_errors = records
        .iter()
        .map(|x| validate_record(x))
        .filter(|x| x.is_err())
        .collect::<Vec<_>>();

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
            Some(Attachment {
                attachment: attachment.as_bytes().to_vec(),
                name: attachment_name.clone(),
                mime: "text/csv; charset=utf8".to_string(),
            }),
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
        Some(Attachment {
            attachment: file,
            name: attachment_name,
            mime: "text/csv; charset=utf8".to_string(),
        }),
        ses_client,
        rt,
    )?;
    Ok(())
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

fn get_file_from_s3(
    bucket: &str,
    key: &str,
    s3_client: &S3Client,
    rt: &mut tokio::runtime::Runtime,
) -> Result<Vec<u8>> {
    info!("Reading bucket: {}, key: {}", bucket, key);
    let s3file_fut = s3_client.get_object(GetObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
        ..Default::default()
    });

    let s3file = rt.block_on(s3file_fut)?;

    let mut buffer: Vec<u8> = Vec::new();
    let _file = s3file
        .body
        .unwrap()
        .into_blocking_read()
        .read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn write_file_to_s3(
    file: Vec<u8>,
    bucket: &str,
    key: &str,
    s3_client: &S3Client,
    rt: &mut tokio::runtime::Runtime,
) -> Result<()> {
    let fut = s3_client.put_object(PutObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
        body: Some(file.into()),
        ..Default::default()
    });
    let _response = rt.block_on(fut)?;
    Ok(())
}

struct Attachment {
    attachment: Vec<u8>,
    name: String,
    mime: String, // "text/csv; charset=utf8"
}

fn send_email(
    recipient: &str,
    subject: &str,
    plain_body: &str,
    html_body: Option<&str>,
    attachment: Option<Attachment>,
    ses_client: &SesClient,
    rt: &mut tokio::runtime::Runtime,
) -> Result<()> {
    let email = Message::builder()
        .to(recipient.parse().unwrap())
        .from(FROM.parse().unwrap())
        .subject(subject);

    // TODO Also simplify to only single part if both HTML and Attachment are None
    let mpart = if html_body.is_some() {
        MultiPart::mixed().multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::quoted_printable()
                        .header(header::ContentType(
                            "text/plain; charset=utf8".parse().unwrap(),
                        ))
                        .body(plain_body),
                )
                .singlepart(
                    SinglePart::eight_bit()
                        .header(header::ContentType(
                            "text/html; charset=utf8".parse().unwrap(),
                        ))
                        .body(html_body.unwrap()),
                ),
        )
    } else {
        MultiPart::mixed().singlepart(
            SinglePart::quoted_printable()
                .header(header::ContentType(
                    "text/plain; charset=utf8".parse().unwrap(),
                ))
                .body(plain_body),
        )
    };
    let mpart = if attachment.is_some() {
        mpart.singlepart(
            SinglePart::base64()
                .header(header::ContentType(
                    attachment.as_ref().unwrap().mime.parse().unwrap(),
                ))
                .header(lettre::message::header::ContentDisposition {
                    disposition: DispositionType::Attachment,
                    parameters: vec![DispositionParam::Filename(
                        Charset::Us_Ascii,
                        None,
                        attachment.as_ref().unwrap().name.as_bytes().to_vec(), // the actual bytes of the filename
                    )],
                })
                .body(attachment.unwrap().attachment),
        )
    } else {
        mpart
    };

    let email = email.multipart(mpart)?;

    let msg_string = email.formatted();
    debug!("Raw email: {}", std::str::from_utf8(&msg_string)?);

    let raw_message = rusoto_ses::RawMessage {
        data: bytes::Bytes::from(base64::encode(msg_string)),
    };
    let request = rusoto_ses::SendRawEmailRequest {
        configuration_set_name: None,
        destinations: None,
        from_arn: None,
        raw_message,
        return_path_arn: None,
        source: None,
        source_arn: None,
        tags: None,
    };

    let fut = ses_client.send_raw_email(request);

    let response = rt.block_on(fut)?;
    info!("Email sent: {:?}", response);

    Ok(())
}
