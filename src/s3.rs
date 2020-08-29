use anyhow::Result;
use log::info;
use rusoto_s3::{GetObjectRequest, PutObjectRequest, S3Client, S3};
use std::io::Read;

pub fn get_file_from_s3(
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

pub fn write_file_to_s3(
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
