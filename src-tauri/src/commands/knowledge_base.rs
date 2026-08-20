use tauri::{State, Emitter};
use std::sync::Arc;
use crate::AppState;
use crate::db::repository::Repository;
use crate::services::knowledge::{repository::KbRepository, models::*};
use serde::Deserialize;

#[tauri::command]
pub async fn get_knowledge_bases(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<KbKnowledgeBase>, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.get_all_kbs().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_knowledge_base(
    state: State<'_, Arc<AppState>>,
    input: CreateKbInput,
) -> Result<KbKnowledgeBase, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.create_kb(&input).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_knowledge_base(
    state: State<'_, Arc<AppState>>,
    id: String,
    input: UpdateKbInput,
) -> Result<KbKnowledgeBase, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.update_kb(&id, &input).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_knowledge_base(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.delete_kb(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_kb_documents(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<Vec<KbDocument>, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.get_documents(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_kb_document(
    state: State<'_, Arc<AppState>>,
    doc_id: String,
    kb_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(state.db.pool.clone());
    let doc = match repo.get_document(&doc_id).await {
        Ok(doc) if doc.kb_id == kb_id => doc,
        Ok(_) => return Err("文档不属于该知识库".to_string()),
        Err(sqlx::Error::RowNotFound) => return Err("文档不存在".to_string()),
        Err(error) => return Err(format!("读取文档失败: {}", error)),
    };
    if doc.source_type == "upload" {
        if let Some(path) = &doc.file_path {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("删除文档文件失败: {}", error)),
            }
        }
    }
    repo.delete_document(&doc_id).await.map_err(|e| e.to_string())?;
    repo.update_kb_counts(&kb_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn reindex_kb_document(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    doc_id: String,
) -> Result<(), String> {
    let pool = state.db.pool.clone();
    crate::services::knowledge::processor::reindex_document(&pool, &app, &doc_id)
        .await
        .map_err(|e| e)
}

#[tauri::command]
pub async fn get_kb_tags(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
    #[allow(unused_variables)]
    limit: Option<usize>,
) -> Result<Vec<KbTag>, String> {
    let pool = &state.db.pool;
    let limit = limit.unwrap_or(15);

    // Sample chunk contents for keyword extraction
    let chunks: Vec<(String,)> = sqlx::query_as(
        "SELECT content FROM kb_chunks WHERE kb_id = ? ORDER BY RANDOM() LIMIT 200"
    )
    .bind(&kb_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if chunks.is_empty() {
        return Ok(vec![]);
    }

    // Simple word frequency analysis
    let mut word_freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // Common stopwords (Chinese + English + code)
    const STOPWORDS: &[&str] = &[
        // English
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "must", "shall", "can", "need",
        "of", "to", "in", "for", "on", "at", "by", "with", "from",
        "as", "into", "through", "during", "before", "after", "above", "below",
        "up", "down", "out", "off", "over", "under", "again", "further",
        "then", "once", "here", "there", "when", "where", "why", "how",
        "all", "each", "every", "both", "few", "more", "most", "other",
        "some", "such", "no", "nor", "not", "only", "own", "same", "so",
        "than", "too", "very", "just", "also", "if", "or", "and", "but",
        // Code / tech common
        "function", "return", "const", "let", "var", "class", "import",
        "export", "default", "pub", "fn", "use", "mod", "struct",
        "impl", "self", "crate", "async", "await", "type", "enum",
        "true", "false", "null", "none", "some", "ok", "err",
        "string", "vec", "option", "result",
        // Chinese
        "的", "了", "在", "是", "我", "有", "和", "就", "不", "人",
        "都", "一", "一个", "上", "也", "很", "到", "说", "要", "去",
        "你", "会", "着", "没有", "看", "好", "自己", "这", "那",
        "与", "或", "但", "而", "且", "则", "于", "以", "及", "为",
        "可", "能", "对", "中", "等", "使", "其", "之", "所",
    ];

    let stopword_set: std::collections::HashSet<&str> = STOPWORDS.iter().copied().collect();

    for (content,) in &chunks {
        // Extract words: English words (2+ chars), Chinese bigrams
        let chars: Vec<char> = content.chars().collect();

        // English words
        let mut current_word = String::new();
        for &ch in &chars {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                current_word.push(ch);
            } else {
                if current_word.len() >= 4 {
                    let word_lower = current_word.to_lowercase();
                    if !stopword_set.contains(word_lower.as_str()) {
                        *word_freq.entry(word_lower).or_insert(0) += 1;
                    }
                }
                current_word.clear();
            }
        }
        if current_word.len() >= 4 {
            let word_lower = current_word.to_lowercase();
            if !stopword_set.contains(word_lower.as_str()) {
                *word_freq.entry(word_lower).or_insert(0) += 1;
            }
        }

        // Chinese bigrams (2-char sequences of CJK characters)
        let mut prev_cjk: Option<char> = None;
        for &ch in &chars {
            let is_cjk = (ch >= '\u{4e00}' && ch <= '\u{9fff}')
                || (ch >= '\u{3400}' && ch <= '\u{4dbf}');
            if is_cjk {
                if let Some(prev) = prev_cjk {
                    let bigram = format!("{}{}", prev, ch);
                    // Filter out bigrams where both chars are common stopwords
                    let prev_s = prev.to_string();
                    let ch_s = ch.to_string();
                    if !stopword_set.contains(prev_s.as_str()) && !stopword_set.contains(ch_s.as_str()) {
                        *word_freq.entry(bigram).or_insert(0) += 1;
                    }
                }
                prev_cjk = Some(ch);
            } else {
                prev_cjk = None;
            }
        }
    }

    // Sort by frequency and take top N
    let mut freq_vec: Vec<(String, usize)> = word_freq.into_iter().collect();
    freq_vec.sort_by(|a, b| b.1.cmp(&a.1));

    let tags: Vec<KbTag> = freq_vec
        .into_iter()
        .take(limit)
        .map(|(word, count)| KbTag { word, count })
        .collect();

    Ok(tags)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KbTag {
    pub word: String,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct KbSearchInput {
    pub query: String,
    pub kb_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub vector_weight: Option<f32>,
    #[serde(default)]
    pub keyword_weight: Option<f32>,
    #[serde(default)]
    pub search_mode: Option<String>,
}

fn default_top_k() -> usize { 5 }

#[tauri::command]
pub async fn search_knowledge_base(
    state: State<'_, Arc<AppState>>,
    input: KbSearchInput,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.db.pool;
    let repo = Repository::new(pool.clone());
    let search_mode = input
        .search_mode
        .as_deref()
        .unwrap_or(if input.kb_id.is_some() { "hybrid" } else { "vector" });
    if !matches!(search_mode, "hybrid" | "vector" | "keyword") {
        return Err(format!("Unsupported search_mode: {}", search_mode));
    }

    if input.kb_id.is_none() && search_mode != "vector" {
        return Err("Cross-knowledge-base search currently supports vector mode only".to_string());
    }

    let vector_weight = input.vector_weight.unwrap_or(0.7);
    let keyword_weight = input.keyword_weight.unwrap_or(0.3);
    if search_mode == "hybrid"
        && (!vector_weight.is_finite()
            || !keyword_weight.is_finite()
            || vector_weight < 0.0
            || keyword_weight < 0.0
            || vector_weight + keyword_weight <= 0.0)
    {
        return Err("Hybrid search weights must be finite, non-negative, and have a positive sum".to_string());
    }

    if search_mode == "keyword" {
        let kb_id = input.kb_id.as_deref().expect("keyword mode requires kb_id");
        return crate::services::knowledge::retriever::keyword_only_search(
            pool,
            kb_id,
            &input.query,
            input.top_k,
        )
        .await;
    }

    let (emb_model, embedding_channel_id) = if let Some(kb_id) = &input.kb_id {
        let kb_repo = KbRepository::new(pool.clone());
        let kb = kb_repo
            .get_kb(kb_id)
            .await
            .map_err(|e| format!("Failed to load knowledge base: {}", e))?;
        (
            kb.embedding_model.unwrap_or_else(|| "text-embedding-3-small".to_string()),
            kb.embedding_channel_id,
        )
    } else {
        ("text-embedding-3-small".to_string(), None)
    };

    let embeddings = crate::services::knowledge::embedder::embed_with_channel(
        &[input.query.clone()], &emb_model, &repo, embedding_channel_id.as_deref()
    ).await.map_err(|e| e)?;

    if embeddings.is_empty() {
        return Err("Failed to embed query".to_string());
    }

    let results = if let Some(kb_id) = &input.kb_id {
        if search_mode == "hybrid" {
            crate::services::knowledge::retriever::hybrid_search(
                pool,
                kb_id,
                &input.query,
                &embeddings[0],
                input.top_k,
                vector_weight,
                keyword_weight,
            )
            .await
        } else {
            crate::services::knowledge::retriever::search(
                pool,
                kb_id,
                &embeddings[0],
                input.top_k,
            )
            .await
        }
    } else {
        crate::services::knowledge::retriever::search_all(pool, &embeddings[0], input.top_k, false).await
    };

    results.map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct KbAskInput {
    pub question: String,
    pub kb_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_chat_model")]
    pub model: String,
    #[serde(default)]
    pub history: Option<Vec<ConversationMessage>>,
    #[serde(default)]
    pub deep_research: bool,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default)]
    pub vector_weight: Option<f32>,
    #[serde(default)]
    pub keyword_weight: Option<f32>,
    #[serde(default)]
    pub search_mode: Option<String>,
}

fn default_chat_model() -> String { "gpt-4o".to_string() }
fn default_max_rounds() -> usize { 5 }

#[tauri::command]
pub async fn ask_knowledge_base(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    input: KbAskInput,
) -> Result<RagAnswer, String> {
    let pool = &state.db.pool;
    let kb_id = input.kb_id.clone().unwrap_or_default();

    let emb_model = if !kb_id.is_empty() {
        let kb_repo = KbRepository::new(pool.clone());
        let kb = kb_repo
            .get_kb(&kb_id)
            .await
            .map_err(|error| format!("Failed to load knowledge base: {}", error))?;
        kb.embedding_model
            .unwrap_or_else(|| "text-embedding-3-small".to_string())
    } else {
        "text-embedding-3-small".to_string()
    };

    if input.deep_research && !kb_id.is_empty() {
        crate::services::knowledge::rag::deep_research(
            pool, &kb_id, &input.question, &emb_model, &input.model,
            input.top_k, input.max_rounds, &app,
        ).await.map_err(|e| e)
    } else {
        let history = input.history.unwrap_or_default();
        let vector_weight = input.vector_weight.unwrap_or(0.7);
        let keyword_weight = input.keyword_weight.unwrap_or(0.3);
        let search_mode = input.search_mode.as_deref().unwrap_or("hybrid");
        crate::services::knowledge::rag::ask_with_config(
            pool, &kb_id, &input.question, &emb_model, &input.model,
            input.top_k, false, &history, &app,
            vector_weight, keyword_weight, search_mode,
        ).await.map_err(|e| e)
    }
}

#[tauri::command]
pub async fn get_kb_stats(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<serde_json::Value, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    let kb = repo.get_kb(&kb_id).await.map_err(|e| e.to_string())?;
    let docs = repo.get_documents(&kb_id).await.map_err(|e| e.to_string())?;
    let ready = docs.iter().filter(|d| d.status == "ready").count();
    let processing = docs.iter().filter(|d| d.status == "processing").count();
    let failed = docs.iter().filter(|d| d.status == "failed").count();

    let index_meta = repo.get_index_meta(&kb_id).await.map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "kb": kb,
        "documents": {
            "total": docs.len(),
            "ready": ready,
            "processing": processing,
            "failed": failed,
        },
        "index": index_meta,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UploadDocInput {
    pub kb_id: String,
    pub filename: String,
    pub content: String, // base64 encoded
}

#[tauri::command]
pub async fn upload_kb_document(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    input: UploadDocInput,
) -> Result<KbDocument, String> {
    use sha2::Digest;
    use tauri::Manager;

    let pool = &state.db.pool;
    let repo = KbRepository::new(pool.clone());

    let content = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD, &input.content
    ).map_err(|e| format!("Invalid base64: {}", e))?;

    let hash = sha2::Sha256::digest(&content);
    let hash_hex = hex::encode(hash);

    match repo.find_document_by_hash(&input.kb_id, &hash_hex).await {
        Ok(Some(_)) => return Err("Document with same content already exists".to_string()),
        Ok(None) => {}
        Err(error) => return Err(error.to_string()),
    }

    crate::services::knowledge::safe_path_component(&input.filename, "filename")?;

    let file_type = crate::services::knowledge::parser::get_file_type(&input.filename);
    let file_size = content.len() as i64;

    let app_data_dir = app.path().app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let kb_dir = app_data_dir.join("kb_files").join(&input.kb_id);
    std::fs::create_dir_all(&kb_dir)
        .map_err(|error| format!("创建知识库文件目录失败: {}", error))?;
    let doc_id = uuid::Uuid::new_v4().to_string();
    let file_path = kb_dir.join(format!("{}_{}", &doc_id, &input.filename));
    std::fs::write(&file_path, &content)
        .map_err(|error| format!("保存文档文件失败: {}", error))?;
    let file_path_str = file_path.to_string_lossy().to_string();

    let doc = match repo.create_document(
        &input.kb_id, &input.filename, Some(&file_path_str),
        &file_type, file_size, &hash_hex
    ).await {
        Ok(doc) => doc,
        Err(error) => {
            if let Err(remove_error) = std::fs::remove_file(&file_path) {
                tracing::warn!(%remove_error, path = %file_path.display(), "failed to remove document after DB insert error");
            }
            return Err(error.to_string());
        }
    };

    let kb = repo.get_kb(&input.kb_id).await.map_err(|e| e.to_string())?;
    let emb_model = kb.embedding_model.clone();

    let pool_clone = pool.clone();
    let app_clone = app.clone();
    let doc_id_clone = doc.id.clone();
    let filename_clone = input.filename.clone();

    tokio::spawn(async move {
        if let Err(e) = crate::services::knowledge::processor::process_document(
            &pool_clone, &app_clone, &input.kb_id, &doc_id_clone,
            &filename_clone, &content, emb_model.as_deref()
        ).await {
            tracing::error!("Document processing failed: {}", e);
        }
    });

    Ok(doc)
}

// ════════════════════════════════════════════════════════
// New commands: Conversations, Sources, Index, Import
// ════════════════════════════════════════════════════════

#[tauri::command]
pub async fn get_kb_conversations(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<Vec<KbConversation>, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.get_conversations(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_kb_conversations(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.clear_conversations(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_kb_sources(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<Vec<KbSource>, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.get_sources(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_kb_source(
    state: State<'_, Arc<AppState>>,
    source_id: String,
    kb_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.delete_source_with_documents(&kb_id, &source_id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => "来源不存在或不属于该知识库".to_string(),
            error => error.to_string(),
        })?;
    repo.update_kb_counts(&kb_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn import_kb_source(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    kb_id: String,
    input: ImportSourceInput,
) -> Result<KbSource, String> {
    let pool = state.db.pool.clone();
    let repo = KbRepository::new(pool.clone());

    let source = repo.create_source(
        &kb_id,
        &input.source_type,
        input.repo_url.as_deref().or(input.url.as_deref()),
        input.dir_path.as_deref(),
        input.branch.as_deref(),
    ).await.map_err(|e| e.to_string())?;

    let source_id = source.id.clone();
    let source_type = input.source_type.clone();

    tokio::spawn(async move {
        let result = if source_type == "git" {
            crate::services::knowledge::importer::import_git_repo(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else if source_type == "url" {
            crate::services::knowledge::importer::import_url(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else if source_type == "local_dir" {
            crate::services::knowledge::importer::import_local_dir(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else {
            Err(format!("Unknown source type: {}", source_type))
        };

        let repo = KbRepository::new(pool.clone());
        match result {
            Ok(count) => {
                if let Err(error) = repo.update_source_status(&source_id, "done", count as i64, None).await {
                    tracing::warn!(%error, source_id = %source_id, "failed to persist knowledge source completion");
                }
            }
            Err(e) => {
                if let Err(error) = repo.update_source_status(&source_id, "error", 0, Some(&e)).await {
                    tracing::warn!(%error, source_id = %source_id, "failed to persist knowledge source failure");
                }
                tracing::error!("Import failed: {}", e);
            }
        }
    });

    Ok(source)
}

#[tauri::command]
pub async fn get_kb_index_status(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<Option<KbIndexMeta>, String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.get_index_meta(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn build_kb_index(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    kb_id: String,
) -> Result<(), String> {
    let pool = state.db.pool.clone();

    // Update status to building immediately
    let repo = KbRepository::new(pool.clone());
    repo.update_kb_index_status(&kb_id, "building")
        .await
        .map_err(|error| format!("Failed to update index status: {}", error))?;

    // Spawn the actual HNSW index build
    tokio::spawn(async move {
        let app = app.clone();
        let kb_id_clone = kb_id.clone();

        // Emit starting event
        let _ = app.emit("kb-index-progress", serde_json::json!({
            "kb_id": &kb_id_clone,
            "status": "building",
            "progress": 0,
            "current": 0,
            "total": 0,
            "message": "正在加载切片数据…"
        }));

        match crate::services::knowledge::retriever::build_index(&pool, &kb_id_clone, &app).await {
            Ok(()) => {
                tracing::info!("HNSW index built successfully for KB {}", kb_id_clone);
                let _ = app.emit("kb-index-progress", serde_json::json!({
                    "kb_id": &kb_id_clone,
                    "status": "ready",
                    "progress": 100,
                    "message": "索引构建完成"
                }));
            }
            Err(e) => {
                tracing::error!("Failed to build HNSW index for KB {}: {}", kb_id_clone, e);
                let repo = KbRepository::new(pool.clone());
                if let Err(error) = repo.update_kb_index_status(&kb_id_clone, "error").await {
                    tracing::warn!(%error, knowledge_base_id = %kb_id_clone, "failed to persist index failure status");
                }
                let _ = app.emit("kb-index-progress", serde_json::json!({
                    "kb_id": &kb_id_clone,
                    "status": "error",
                    "message": format!("索引构建失败: {}", e)
                }));
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn drop_kb_index(
    state: State<'_, Arc<AppState>>,
    kb_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(state.db.pool.clone());
    repo.upsert_index_meta(&kb_id, 0, 0, None, "none").await.map_err(|e| e.to_string())?;
    repo.update_kb_index_status(&kb_id, "none").await.map_err(|e| e.to_string())?;
    Ok(())
}
