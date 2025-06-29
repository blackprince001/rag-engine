use crate::server::errors::AppError;
use crate::server::serializers::{AppState, FileResponse, Pagination};
use crate::service::scheduler::FileEventType;

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::db::{
    get_database_connection,
    models::{self, NewFile},
};

pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::FileUploadError(format!("Failed to process form: {}", e)))?
    {
        let file_name = field
            .file_name()
            .ok_or(AppError::FileUploadError(
                "File name not provided".to_string(),
            ))?
            .to_string();

        let content_type = field.content_type().map(|ct| ct.to_string());

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::FileUploadError(format!("Failed to read file data: {}", e)))?;

        let file_id = Uuid::new_v4();
        let file_path = state.upload_dir.join(file_id.to_string());

        let mut file = File::create(&file_path)
            .await
            .map_err(|e| AppError::FileUploadError(format!("Failed to create file: {}", e)))?;

        file.write_all(&data)
            .await
            .map_err(|e| AppError::FileUploadError(format!("Failed to write file: {}", e)))?;

        let mut hasher = Sha256::new();
        hasher.update(&data);
        let file_hash = format!("{:x}", hasher.finalize());

        let new_file = NewFile {
            file_path: file_path.to_string_lossy().to_string(),
            file_name,
            file_size: Some(data.len() as i64),
            file_type: content_type,
            file_hash: Some(file_hash),
            metadata: None,
        };

        let mut conn = get_database_connection()
            .map_err(|e| AppError::DatabaseError(format!("Could not connect to database: {}", e)))
            .unwrap();

        let result = models::File::create_file(&mut conn, new_file)
            .map_err(|e| {
                AppError::DatabaseError(format!("File could not be created details: {}", e))
            })
            .unwrap();

        let file_type = result.file_type.expect("File does not have a type yet!");

        let _ = state
            .scheduler
            .schedule_file(
                file_id,
                result.file_path,
                file_type.clone(),
                FileEventType::Created,
            )
            .await;

        return Ok((
            StatusCode::CREATED,
            Json(FileResponse {
                id: file_id,
                file_name: result.file_name,
                file_type: Some(file_type),
                file_size: result.file_size,
                file_hash: result.file_hash,
            }),
        ));
    }

    Err(AppError::FileUploadError("No file provided".to_string()))
}

pub async fn list_files(pagination: Query<Pagination>) -> Result<impl IntoResponse, AppError> {
    let mut conn = get_database_connection()
        .map_err(|e| AppError::DatabaseError(format!("Could not connect to database: {}", e)))
        .unwrap();

    let results = models::File::find_files(&mut conn, pagination.skip, pagination.limit)
        .map_err(|e| AppError::NotFoundError(format!("{}", e)))
        .unwrap();

    let response: Vec<FileResponse> = results
        .into_iter()
        .map(|file| FileResponse {
            id: file.id,
            file_name: file.file_name,
            file_type: file.file_type,
            file_size: file.file_size,
            file_hash: file.file_hash,
        })
        .collect();

    return Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "files": response,
            "meta": {
                "offset": pagination.skip,
                "limit": pagination.limit,
                "total": response.len()
            }
        }
        )),
    ));
}

// Process a previously uploaded file
pub async fn process_file(
    // State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = get_database_connection()
        .map_err(|e| AppError::DatabaseError(format!("Could not connect to database: {}", e)))
        .unwrap();

    let file = models::File::find_file(&mut conn, id)
        .map_err(|e| AppError::DatabaseError(format!("Failed to find file: {}", e)))?;

    // Here you would call your existing logic to process the file
    // This is where you'd implement the PDF extraction, chunking, and DB storage
    // process_document(&file).await?;

    Ok(Json(serde_json::json!({
        "message": "File processing started",
        "id": file.id
    })))
}
