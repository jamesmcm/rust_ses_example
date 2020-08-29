use anyhow::Result;
use lettre::message::header::{Charset, DispositionParam, DispositionType};
use lettre::message::{header, Message, MultiPart, SinglePart};
use log::{debug, info};
use rusoto_ses::Ses;
use rusoto_ses::SesClient;

pub struct Attachment {
    attachment: Vec<u8>,
    name: String,
    mime: String, // "text/csv; charset=utf8"
}

impl Attachment {
    pub fn new(attachment: Vec<u8>, name: String, mime: String) -> Self {
        Self {
            attachment,
            name,
            mime,
        }
    }
}

pub fn send_email(
    recipient: &str,
    subject: &str,
    plain_body: &str,
    html_body: Option<&str>,
    attachment: Option<Attachment>,
    ses_client: &SesClient,
    rt: &mut tokio::runtime::Runtime,
) -> Result<()> {
    // Warning: Note global FROM variable
    let email = Message::builder()
        .to(recipient.parse().unwrap())
        .from(crate::FROM.parse().unwrap())
        .subject(subject);

    // TODO Also simplify to only single part if both HTML and Attachment are None
    let mpart = if let Some(html_body) = html_body {
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
                        .body(html_body),
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
    let mpart = if let Some(attachment) = attachment {
        mpart.singlepart(
            SinglePart::base64()
                .header(header::ContentType(attachment.mime.parse().unwrap()))
                .header(lettre::message::header::ContentDisposition {
                    disposition: DispositionType::Attachment,
                    parameters: vec![DispositionParam::Filename(
                        Charset::Us_Ascii,
                        None,
                        attachment.name.as_bytes().to_vec(), // the actual bytes of the filename
                    )],
                })
                .body(attachment.attachment),
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
