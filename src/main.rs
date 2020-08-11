use anyhow::Result;
use csv::Writer;
use lambda_runtime::error::HandlerError;
use mailparse::parse_mail;
use percent_encoding::percent_decode_str;
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, PutObjectRequest, S3Client, S3};
use serde::Deserialize;
use serde::Serialize;
use std::error::Error;
use std::io::Cursor;
use std::io::Read;

fn main() -> Result<()> {
    lambda_runtime::lambda!(my_handler);

    Ok(())
}

// Use serde enum to handle separate events?
// https://serde.rs/enum-representations.html

fn my_handler(
    e: aws_lambda_events::event::s3::S3Event,
    _c: lambda_runtime::Context,
) -> Result<(), HandlerError> {
    println!("{:?}", e);
    let decodedkey = percent_decode_str(&(e.records[0].s3.object.key.as_ref()).unwrap())
        .decode_utf8()
        .unwrap();

    match handle_email(&decodedkey) {
        Ok(_) => (),
        Err(error) => {
            panic!("Error: {:?}", error);
        }
    }

    Ok(())
}

fn handle_email(key: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lettre::message::header::{Charset, ContentDisposition, DispositionParam, DispositionType};
    use lettre::message::{header, Message, MultiPart, Part, SinglePart};
    use rusoto_core::Region;
    use rusoto_ses::Ses;
    use rusoto_ses::SesClient;
    use std::fs::read_to_string;

    #[derive(Debug, Deserialize)]
    struct Record {
        #[serde(rename = "Rank")]
        rank: u32,
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Platform")]
        platform: String,
        #[serde(rename = "Year")]
        year: String,
    }

    #[test]
    fn test_get_subject() -> Result<()> {
        let file = read_to_string("test/onlytext.txt")?;
        let cap = parse_mail(file.as_bytes())?;
        assert_eq!(
            cap.headers
                .iter()
                .filter(|x| x.get_key() == "Subject")
                .map(|x| x.get_value())
                .next()
                .unwrap(),
            "testmail"
        );
        Ok(())
    }

    #[test]
    fn test_text_attachment() -> Result<()> {
        let file = read_to_string("test/plaintext_attachment.txt")?;
        let cap = parse_mail(file.as_bytes())?;
        let attachment = cap
            .subparts
            .into_iter()
            .filter(|x| {
                x.get_content_disposition().disposition == mailparse::DispositionType::Attachment
            })
            .next()
            .expect("No attachment")
            .get_body()?;

        assert_eq!(attachment.trim(), "plaintext");
        Ok(())
    }

    #[test]
    fn test_large_csv() -> Result<()> {
        let file = read_to_string("test/largecsv.txt")?;
        let cap = parse_mail(file.as_bytes())?;
        let attachment = cap
            .subparts
            .into_iter()
            .filter(|x| {
                x.get_content_disposition().disposition == mailparse::DispositionType::Attachment
            })
            .next()
            .expect("No attachment")
            .get_body()?;
        let mut rdr = csv::Reader::from_reader(Cursor::new(attachment.trim()));
        let mut records: Vec<Record> = Vec::with_capacity(5);
        for result in rdr.deserialize() {
            let record: Record = result?;
            records.push(record);
            break;
        }
        assert_eq!(records[0].name, "Wii Sports");
        Ok(())
    }

    #[tokio::test]
    async fn send_email() -> Result<()> {
        let email = Message::builder()
            // Addresses can be specified by the tuple (email, alias)
            .to("James McMurray <jamesmcm03@gmail.com>".parse().unwrap())
            // ... or by an address only
            .from("SES Test <test@testjamesmcm.awsapps.com>".parse().unwrap())
            .subject("Hi, Hello world")
            .multipart(
                MultiPart::mixed()
                    .multipart(
                        MultiPart::alternative()
                            .singlepart(
                                SinglePart::quoted_printable()
                                    .header(header::ContentType(
                                        "text/plain; charset=utf8".parse().unwrap(),
                                    ))
                                    .body("Email text2"),
                            )
                            .singlepart(
                                SinglePart::eight_bit()
                                    .header(header::ContentType(
                                        "text/html; charset=utf8".parse().unwrap(),
                                    ))
                                    .body("<p><b>Email</b>, <i>text2</i>!</p>"),
                            ),
                    )
                    .singlepart(
                        SinglePart::base64()
                            .header(header::ContentType(
                                "text/plain; charset=utf8".parse().unwrap(),
                            ))
                            .header(lettre::message::header::ContentDisposition {
                                disposition: DispositionType::Attachment,
                                parameters: vec![DispositionParam::Filename(
                                    Charset::Us_Ascii,
                                    None, // The optional language tag (see `language-tag` crate)
                                    b"attachedfile2.txt".to_vec(), // the actual bytes of the filename
                                )],
                            })
                            .body("plaintext"),
                    ),
            )?;

        let msg_string = email.formatted();
        println!("{}", std::str::from_utf8(&msg_string)?);

        let ses_client = SesClient::new(Region::EuWest1);
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

        let response = ses_client.send_raw_email(request).await;

        println!("{:?}", response?);

        Ok(())
    }
}
