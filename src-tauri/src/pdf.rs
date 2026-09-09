//! PDF 合并与拆分命令（lopdf，纯本地处理）

use crate::images::JobProgress;
use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMeta {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub pages: u32,
    pub encrypted: bool,
    pub title: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMergeResult {
    pub out_path: String,
    pub pages: u32,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfSplitResult {
    pub name: String,
    pub out_path: Option<String>,
    pub pages: u32,
    pub size: u64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPart {
    pub name: String,
    /// 1-based 页码列表
    pub pages: Vec<u32>,
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 读取 PDF 元信息（页数、大小、加密状态等，仅用于预览）
#[tauri::command]
pub async fn pdf_meta(paths: Vec<String>) -> Result<Vec<PdfMeta>, String> {
    tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|p| meta_one(&p)).collect())
        .await
        .map_err(|e| e.to_string())
}

fn meta_one(path: &str) -> PdfMeta {
    let p = Path::new(path);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    match Document::load_metadata(p) {
        Ok(m) => PdfMeta {
            path: path.to_string(),
            name,
            size,
            pages: m.page_count,
            encrypted: m.encrypted,
            title: m.title,
            error: None,
        },
        Err(e) => PdfMeta {
            path: path.to_string(),
            name,
            size,
            pages: 0,
            encrypted: false,
            title: None,
            error: Some(format!("@err_pdf_read|{e}")),
        },
    }
}

/// 合并核心逻辑（lopdf 官方 merge 示例的简化实现），进度通过回调上报。
fn merge_pdfs_core(
    paths: &[String],
    out_path: &str,
    progress: &dyn Fn(u32, u32, Option<&str>),
) -> Result<(u32, u64), String> {
    let total = paths.len() as u32;
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut document = Document::with_version("1.5");
    let mut max_id = 1u32;

    for (i, path) in paths.iter().enumerate() {
        progress((i + 1) as u32, total, Some(&format!("@msg_reading_file|{}", file_label(path))));
        let mut doc =
            Document::load(path).map_err(|e| format!("@err_pdf_read_file|{}|{e}", file_label(path)))?;
        if doc.is_encrypted() {
            return Err(format!("@err_pdf_encrypted_merge|{}", file_label(path)));
        }
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        // 物化 Pages 级继承属性到每个页面（Resources/MediaBox/CropBox/Rotate），
        // 避免多文档合并后页面继承错文档的资源（cairo/WeasyPrint 等生成器常见）
        let inherited: Vec<(Vec<u8>, Object)> = {
            let mut v = Vec::new();
            if let Ok(root_id) = doc
                .catalog()
                .and_then(|c| c.get(b"Pages"))
                .and_then(Object::as_reference)
            {
                if let Ok(root) = doc.get_dictionary(root_id) {
                    for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
                        if let Ok(val) = root.get(key) {
                            v.push((key.to_vec(), val.clone()));
                        }
                    }
                }
            }
            v
        };
        let pages = doc.get_pages();
        for (_, object_id) in pages {
            if let Ok(obj) = doc.get_object(object_id) {
                let mut page_obj = obj.clone();
                if !inherited.is_empty() {
                    if let Ok(dict) = page_obj.as_dict_mut() {
                        for (k, v) in &inherited {
                            if !dict.has(k) {
                                dict.set(k.clone(), v.clone());
                            }
                        }
                    }
                }
                documents_pages.insert(object_id, page_obj);
            }
        }
        documents_objects.extend(doc.objects);
    }

    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;
    for (object_id, object) in documents_objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object {
                        id
                    } else {
                        object_id
                    },
                    object,
                ));
            }
            b"Pages" => {
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref obj)) = pages_object {
                        if let Ok(old_dictionary) = obj.as_dict() {
                            dictionary.extend(old_dictionary);
                        }
                    }
                    pages_object = Some((
                        if let Some((id, _)) = pages_object {
                            id
                        } else {
                            object_id
                        },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                document.objects.insert(object_id, object);
            }
        }
    }

    let (catalog_id, catalog_object) = catalog_object.ok_or("@err_pdf_no_catalog")?;
    let (pages_id, pages_object) = pages_object.ok_or("@err_pdf_no_pages")?;

    let page_ids: Vec<ObjectId> = documents_pages.keys().cloned().collect();
    let actual_pages = page_ids.len() as u32;
    for (object_id, object) in documents_pages {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_id);
            document
                .objects
                .insert(object_id, Object::Dictionary(dictionary));
        }
    }
    if let Ok(dictionary) = pages_object.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Count", actual_pages);
        dictionary.set(
            "Kids",
            page_ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
        );
        document
            .objects
            .insert(pages_id, Object::Dictionary(dictionary));
    }
    if let Ok(dictionary) = catalog_object.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_id);
        dictionary.remove(b"Outlines");
        document
            .objects
            .insert(catalog_id, Object::Dictionary(dictionary));
    }
    document.trailer.set("Root", catalog_id);
    document.max_id = document.objects.len() as u32;
    document.renumber_objects();

    progress(total, total, Some("@msg_writing"));
    document.compress();
    document.save(out_path).map_err(|e| format!("@err_pdf_write|{e}"))?;

    let size = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    Ok((actual_pages, size))
}

/// 合并多个 PDF 为一个
#[tauri::command]
pub async fn pdf_merge(
    app: AppHandle,
    job_id: String,
    paths: Vec<String>,
    out_path: String,
) -> Result<PdfMergeResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len() as u32;
        let cb = |cur: u32, total: u32, msg: Option<&str>| {
            let _ = app.emit(
                "job",
                JobProgress {
                    job_id: job_id.clone(),
                    op: "merge".into(),
                    current: cur,
                    total,
                    message: msg.map(|s| s.to_string()),
                    done: false,
                },
            );
        };
        let (pages, size) = merge_pdfs_core(&paths, &out_path, &cb)?;
        // 全部完成后再发一条终态事件
        let _ = app.emit(
            "job",
            JobProgress {
                job_id,
                op: "merge".into(),
                current: total,
                total,
                message: None,
                done: true,
            },
        );
        Ok(PdfMergeResult {
            out_path,
            pages,
            size,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 拆分核心逻辑：每个 SplitPart 输出一个文件，进度通过回调上报。
fn split_pdfs_core(
    path: &str,
    parts: &[SplitPart],
    out_dir: &str,
    progress: &dyn Fn(u32, u32, Option<&str>),
) -> Result<Vec<PdfSplitResult>, String> {
    let total = parts.len() as u32;
    let doc = Document::load(path).map_err(|e| format!("@err_pdf_read|{e}"))?;
    if doc.is_encrypted() {
        return Err("@err_pdf_encrypted_split".to_string());
    }
    // 输出目录为空时默认使用源文件所在目录
    let out_dir = if out_dir.trim().is_empty() {
        Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string())
    } else {
        out_dir.to_string()
    };
    let pages = doc.get_pages();
    let page_total = pages.len() as u32;
    let mut results = Vec::with_capacity(parts.len());
    // 批内输出名去重（小写），防止同名输出互相覆盖
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (i, part) in parts.iter().enumerate() {
        let current = (i + 1) as u32;
        progress(current, total, Some(&part.name));

        // 校验输出文件名（防御路径穿越）
        if part.name.trim().is_empty()
            || part.name.contains(['/', '\\'])
            || part.name.contains("..")
        {
            results.push(PdfSplitResult {
                name: part.name.clone(),
                out_path: None,
                pages: 0,
                size: 0,
                ok: false,
                error: Some("@err_bad_name".to_string()),
            });
            continue;
        }

        // 校验并整理页码（排序去重）
        let mut pns: Vec<u32> = part.pages.to_vec();
        pns.sort_unstable();
        pns.dedup();
        if pns.is_empty() {
            results.push(PdfSplitResult {
                name: part.name.clone(),
                out_path: None,
                pages: 0,
                size: 0,
                ok: false,
                error: Some("@err_empty_pages".to_string()),
            });
            continue;
        }
        let mut ids: Vec<ObjectId> = Vec::with_capacity(pns.len());
        let mut bad = None;
        for pn in &pns {
            match pages.get(pn) {
                Some(id) => ids.push(*id),
                None => {
                    bad = Some(format!("@err_page_out_of_range|{pn}|{page_total}"));
                    break;
                }
            }
        }
        if let Some(e) = bad {
            results.push(PdfSplitResult {
                name: part.name.clone(),
                out_path: None,
                pages: 0,
                size: 0,
                ok: false,
                error: Some(e),
            });
            continue;
        }

        // 克隆源文档，裁剪页面树
        let mut out = doc.clone();
        let pages_root_id = match out.catalog().and_then(|c| c.get(b"Pages")).and_then(Object::as_reference) {
            Ok(id) => id,
            Err(e) => {
                results.push(PdfSplitResult {
                    name: part.name.clone(),
                    out_path: None,
                    pages: 0,
                    size: 0,
                    ok: false,
                    error: Some(format!("@err_page_tree|{e}")),
                });
                continue;
            }
        };
        {
            let pages_dict = match out.get_object_mut(pages_root_id).and_then(Object::as_dict_mut) {
                Ok(d) => d,
                Err(e) => {
                    results.push(PdfSplitResult {
                        name: part.name.clone(),
                        out_path: None,
                        pages: 0,
                        size: 0,
                        ok: false,
                        error: Some(format!("@err_page_tree|{e}")),
                    });
                    continue;
                }
            };
            pages_dict.set(
                "Kids",
                ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
            );
            pages_dict.set("Count", ids.len() as u32);
        }
        // 移除可能指向已删除页面的书签大纲
        if let Ok(cat) = out.catalog_mut() {
            cat.remove(b"Outlines");
        }

        // 输出路径：批内同名自动加序号，且不覆盖磁盘上已存在的文件
        let mut out_path = Path::new(&out_dir).join(&part.name);
        {
            let key0 = out_path.to_string_lossy().to_lowercase();
            if used_names.contains(&key0) || out_path.exists() {
                let parent = out_path.parent().map(|p| p.to_path_buf());
                let stem = out_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "part".into());
                let ext = out_path
                    .extension()
                    .map(|s| format!(".{}", s.to_string_lossy()))
                    .unwrap_or_default();
                let mut k = 1u32;
                loop {
                    let candidate = format!("{stem} ({k}){ext}");
                    let cp = parent.as_deref().unwrap_or(Path::new(".")).join(&candidate);
                    let ck = cp.to_string_lossy().to_lowercase();
                    if !cp.exists() && !used_names.contains(&ck) {
                        out_path = cp;
                        break;
                    }
                    k += 1;
                }
            }
            used_names.insert(out_path.to_string_lossy().to_lowercase());
        }
        let actual_name = out_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| part.name.clone());

        if let Err(e) = out.save(&out_path) {
            results.push(PdfSplitResult {
                name: actual_name,
                out_path: None,
                pages: ids.len() as u32,
                size: 0,
                ok: false,
                error: Some(format!("@err_pdf_write|{e}")),
            });
            continue;
        }
        let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
        results.push(PdfSplitResult {
            name: actual_name,
            out_path: Some(out_path.to_string_lossy().into_owned()),
            pages: ids.len() as u32,
            size,
            ok: true,
            error: None,
        });
    }

    progress(total, total, None);
    Ok(results)
}

/// 拆分 PDF：每个 SplitPart 输出一个文件
#[tauri::command]
pub async fn pdf_split(
    app: AppHandle,
    job_id: String,
    path: String,
    parts: Vec<SplitPart>,
    out_dir: String,
) -> Result<Vec<PdfSplitResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let total = parts.len() as u32;
        let cb = |cur: u32, total: u32, msg: Option<&str>| {
            let _ = app.emit(
                "job",
                JobProgress {
                    job_id: job_id.clone(),
                    op: "split".into(),
                    current: cur,
                    total,
                    message: msg.map(|s| s.to_string()),
                    done: false,
                },
            );
        };
        let results = split_pdfs_core(&path, &parts, &out_dir, &cb)?;
        // 全部完成后再发一条终态事件
        let _ = app.emit(
            "job",
            JobProgress {
                job_id,
                op: "split".into(),
                current: total,
                total,
                message: None,
                done: true,
            },
        );
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;
    use lopdf::Stream;
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        let d = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 用 lopdf 生成一个含单页文本的合法 PDF（参考官方示例）
    fn make_doc(text: &str) -> Document {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc
    }

    #[test]
    fn merge_then_split_roundtrip() {
        let dir = test_dir();
        let a = dir.join("test_a.pdf");
        let b = dir.join("test_b.pdf");
        let mut d1 = make_doc("Page A");
        d1.save(&a).unwrap();
        let mut d2 = make_doc("Page B");
        d2.save(&b).unwrap();

        let merged = dir.join("test_merged.pdf");
        let (pages, size) = merge_pdfs_core(
            &[
                a.to_string_lossy().into_owned(),
                b.to_string_lossy().into_owned(),
            ],
            &merged.to_string_lossy().into_owned(),
            &|_, _, _| {},
        )
        .unwrap();
        assert_eq!(pages, 2);
        assert!(size > 0);

        let doc = Document::load(&merged).unwrap();
        assert_eq!(doc.get_pages().len(), 2);

        let parts = vec![
            SplitPart {
                name: "test_part1.pdf".into(),
                pages: vec![1],
            },
            SplitPart {
                name: "test_part2.pdf".into(),
                pages: vec![2],
            },
        ];
        let results = split_pdfs_core(
            &merged.to_string_lossy().into_owned(),
            &parts,
            &dir.to_string_lossy().into_owned(),
            &|_, _, _| {},
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.ok));

        let p1 = Document::load(dir.join("test_part1.pdf")).unwrap();
        let p2 = Document::load(dir.join("test_part2.pdf")).unwrap();
        assert_eq!(p1.get_pages().len(), 1);
        assert_eq!(p2.get_pages().len(), 1);
    }

    #[test]
    fn split_rejects_out_of_range() {
        let dir = test_dir();
        let a = dir.join("test_range.pdf");
        let mut d1 = make_doc("Only one page");
        d1.save(&a).unwrap();
        let parts = vec![SplitPart {
            name: "bad.pdf".into(),
            pages: vec![1, 99],
        }];
        let results = split_pdfs_core(
            &a.to_string_lossy().into_owned(),
            &parts,
            &dir.to_string_lossy().into_owned(),
            &|_, _, _| {},
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].ok);
    }

    #[test]
    fn split_dedups_duplicate_names_and_never_overwrites() {
        let dir = test_dir();
        let a = dir.join("test_dup.pdf");
        let mut d1 = make_doc("dup");
        d1.save(&a).unwrap();
        let parts = vec![
            SplitPart {
                name: "same.pdf".into(),
                pages: vec![1],
            },
            SplitPart {
                name: "same.pdf".into(),
                pages: vec![1],
            },
        ];
        let results = split_pdfs_core(
            &a.to_string_lossy().into_owned(),
            &parts,
            &dir.to_string_lossy().into_owned(),
            &|_, _, _| {},
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].ok && results[1].ok);
        let p0 = results[0].out_path.as_ref().unwrap();
        let p1 = results[1].out_path.as_ref().unwrap();
        assert_ne!(p0, p1, "同名输出必须自动改名");
        assert!(std::path::Path::new(p0).exists());
        assert!(std::path::Path::new(p1).exists());
    }
}
