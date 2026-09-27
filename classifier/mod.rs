use std::path::Path;

#[cfg(feature = "local-ai")]
use fastembed::{InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel};

#[cfg(feature = "local-ai")]
const EXAMPLES: &[(&str, &str)] = &[
    ("development", "software programming and coding in an IDE"),
    ("development", "terminal based development working on a Git repository"),
    ("development", "debugging and building a software project"),
    ("research", "researching information in a web browser"),
    ("research", "reading scientific papers and PDF documents"),
    ("research", "reading documentation and learning technical information"),
    ("communication", "writing email messages and chatting with colleagues"),
    ("communication", "video meetings and online conversations"),
    ("communication", "team communication in Discord or Slack"),
    ("design", "creating graphics and illustrations"),
    ("design", "working in a visual design editor"),
    ("design", "editing photographs and 3D models"),
    ("music", "listening to music and playlists"),
    ("music", "playing songs and audio albums"),
    ("music", "managing a music library"),
    ("media", "watching movies and streaming video"),
    ("media", "playing online videos and television"),
    ("media", "watching a film in a media player"),
    ("gaming", "playing video games"),
    ("gaming", "browsing the Steam games library"),
    ("gaming", "launching a computer game"),
    ("files", "organizing files and folders"),
    ("files", "browsing the file manager"),
    ("files", "managing local storage and directories"),
    ("system", "configuring system settings"),
    ("system", "monitoring processes and system resources"),
    ("system", "administering the computer"),
    ("office", "writing documents and presentations"),
    ("office", "editing spreadsheets and productivity files"),
    ("office", "taking notes for office work"),
    ("shopping", "shopping online and comparing products"),
    ("shopping", "checking out a shopping cart"),
    ("shopping", "browsing an online store"),
    ("social", "social networking and reading friends' posts"),
    ("social", "viewing a social media feed"),
    ("social", "participating in an online community"),
    ("other", "using a miscellaneous application"),
    ("other", "working in an uncategorized utility"),
    ("other", "unknown general computer activity"),
];

pub struct LocalClassifier {
    #[cfg(feature = "local-ai")]
    model: TextEmbedding,
    #[cfg(feature = "local-ai")]
    examples: Vec<Vec<f32>>,
}

impl LocalClassifier {
    pub fn load(directory: &Path) -> Result<Option<Self>, String> {
        if !directory.join("model.onnx").is_file() { return Ok(None); }
        #[cfg(not(feature = "local-ai"))]
        { return Err("local model installed, but binary was compiled without local-ai".into()); }
        #[cfg(feature = "local-ai")]
        {
            // No hub client or built-in-model constructor is used: all inference inputs are local.
            let file = |name: &str| std::fs::read(directory.join(name))
                .map_err(|e| format!("model {}: {e}", directory.join(name).display()));
            let files = TokenizerFiles {
                tokenizer_file: file("tokenizer.json")?, config_file: file("config.json")?,
                special_tokens_map_file: file("special_tokens_map.json")?,
                tokenizer_config_file: file("tokenizer_config.json")?,
            };
            let onnx = file("model.onnx")?;
            let result = std::panic::catch_unwind(|| {
                let mut model = TextEmbedding::try_new_from_user_defined(
                    UserDefinedEmbeddingModel::new(onnx, files).with_pooling(Pooling::Mean),
                    InitOptionsUserDefined::new().with_max_length(256),
                ).map_err(|e| e.to_string())?;
                let mut examples = model.embed(EXAMPLES.iter().map(|(_, text)| *text).collect::<Vec<&str>>(), None)
                    .map_err(|e| e.to_string())?;
                for vector in &mut examples {
                    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                    if norm > 1e-6 { for value in vector { *value /= norm; } }
                }
                Ok::<_, String>(Self { model, examples })
            });
            result.map_err(|_| "invalid local MiniLM model/tokenizer".to_string())?
                .map(Some)
        }
    }

    pub fn classify(&mut self, contexts: &[&str], minimum_confidence: f32) -> Option<(String, f32)> {
        #[cfg(not(feature = "local-ai"))]
        { let _ = (contexts, minimum_confidence); None }
        #[cfg(feature = "local-ai")]
        {
            let vectors = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.model.embed(contexts.to_vec(), None)
            })).ok()?.ok()?;
            let mut best = ("other", -1.0_f32);
            for vector in &vectors {
                let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm < 1e-6 { continue; }
                for ((category, _), example) in EXAMPLES.iter().zip(&self.examples) {
                    let similarity = vector.iter().zip(example).map(|(a,b)| a * b).sum::<f32>() / norm;
                    if similarity > best.1 { best = (category, similarity); }
                }
            }
            if best.1 < minimum_confidence { return None; }
            Some((best.0.to_owned(), best.1.clamp(0.0, 1.0)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_model_stays_offline() {
        assert!(LocalClassifier::load(Path::new("/nonexistent/oma-smartspaces-model" )).unwrap().is_none());
    }
}
