// inspired by https://github.com/datafuselabs/databend/blob/main/src/query/sharing_endpoint/src/handlers.rs

use poem::web::Json;
use poem::web::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RequestFile {
    pub file_name: String,
    pub method: String,
}

pub async fn translate_files(
    Path((_a, _b, _c)): Path<(String, String, String)>,
    Json(request_files): Json<Vec<RequestFile>>,
) -> Json<Vec<RequestFile>> {
    let vr: Vec<RequestFile> = request_files.clone();
    Json(vr)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_translate_files() {
        let rf = RequestFile {
            file_name: "file1.txt".to_string(),
            method: "GET".to_string(),
        };
        let request_files = vec![rf.clone()];

        let json_request_files: Json<Vec<RequestFile>> = Json(request_files.clone());
        let response = translate_files(
            Path(("tenant1".to_string(), "share1".to_string(), "table1".to_string())),
            json_request_files,
        ).await;

        let response_files: Vec<RequestFile> = response.0;
        let response_file1 = response_files.get(0).unwrap();

        assert_eq!(response_file1.file_name, rf.file_name);
        assert_eq!(response_file1.method, rf.method);
    }
}
