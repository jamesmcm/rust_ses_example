#[cfg(test)]
use super::*;
use crate::csv_serde::Entry;
use lettre::message::header::{Charset, DispositionParam, DispositionType};
use lettre::message::{header, Message, MultiPart, SinglePart};
use rusoto_core::Region;
use rusoto_ses::Ses;
use rusoto_ses::SesClient;
use std::fs::read_to_string;
use std::io::Cursor;

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
        .find(|x| x.get_content_disposition().disposition == mailparse::DispositionType::Attachment)
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
        .find(|x| x.get_content_disposition().disposition == mailparse::DispositionType::Attachment)
        .expect("No attachment")
        .get_body()?;
    let mut rdr = csv::Reader::from_reader(Cursor::new(attachment.trim()));
    let mut records: Vec<Record> = Vec::with_capacity(5);

    let mut result = rdr.deserialize();
    let record: Record = result.next().unwrap()?;
    records.push(record);

    assert_eq!(records[0].name, "Wii Sports");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn send_email() -> Result<()> {
    let email = Message::builder()
        // Addresses can be specified by the tuple (email, alias)
        .to(RECIPIENT.parse().unwrap())
        // ... or by an address only
        .from(FROM.parse().unwrap())
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

#[test]
fn deserialize_with_date() -> Result<()> {
    let attachment = read_to_string("test/testcsv_date.csv")?;
    let mut rdr = csv::Reader::from_reader(Cursor::new(attachment.trim()));
    let mut records: Vec<Entry> = Vec::with_capacity(16);

    for result in rdr.deserialize() {
        records.push(result?);
    }
    assert_eq!(
        records[0],
        Entry {
            id: 1,
            start_date: chrono::naive::NaiveDateTime::parse_from_str(
                "2020-08-23 09:00:00",
                "%Y-%m-%d %H:%M:%S",
            )?,
            end_date: chrono::naive::NaiveDateTime::parse_from_str(
                "2020-08-23 17:00:00",
                "%Y-%m-%d %H:%M:%S",
            )?,
        },
    );
    Ok(())
}
