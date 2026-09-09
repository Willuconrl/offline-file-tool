//! 文件选择对话框与批量重命名相关命令

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

// ---------- 对话框 ----------

fn file_paths_to_strings(list: Vec<FilePath>) -> Vec<String> {
    list.into_iter()
        .filter_map(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// 打开多选文件对话框。filters: [[名称, [扩展名...]], ...]
#[tauri::command]
pub async fn pick_files(
    app: AppHandle,
    filters: Vec<(String, Vec<String>)>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app.dialog().file();
        for (name, exts) in &filters {
            let refs: Vec<&str> = exts.iter().map(String::as_str).collect();
            builder = builder.add_filter(name, &refs);
        }
        builder
            .blocking_pick_files()
            .map(file_paths_to_strings)
            .unwrap_or_default()
    })
    .await
    .map_err(|e| e.to_string())
}

/// 打开目录选择对话框
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|f| f.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

/// 打开保存文件对话框
#[tauri::command]
pub async fn pick_save_path(
    app: AppHandle,
    default_name: String,
    default_dir: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app.dialog().file();
        if let Some(dir) = default_dir.filter(|d| !d.is_empty()) {
            builder = builder.set_directory(dir);
        }
        builder = builder.set_file_name(&default_name);
        builder
            .blocking_save_file()
            .and_then(|f| f.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

/// 在资源管理器中定位文件
#[tauri::command]
pub fn open_in_folder(path: String) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("@err_open_explorer|{e}"))
}

/// 用系统默认程序打开网页或邮件链接（仅允许 http/https/mailto）
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let ok = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("mailto:");
    if !ok || url.contains(char::is_whitespace) {
        return Err("invalid url".to_string());
    }
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("@err_open_explorer|{e}"))
}

/// 读取单个文件的基础元信息
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[tauri::command]
pub fn file_meta(path: String) -> Result<FileMeta, String> {
    let p = Path::new(&path);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    Ok(FileMeta { path, name, size })
}

// ---------- 批量重命名 ----------

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct RenameRules {
    /// 文件名模板（不含扩展名）：{name} 原名、{upper}/{lower} 大小写、
    /// {num}/{num:3} 序号、{date}/{date:yyyy-MM-dd} 日期；空模板等价于 {name}
    pub template: String,
    pub num_start: i64,
    pub num_step: i64,
    /// {date} 无参数时的默认格式
    pub date_format: String,
    pub find: String,
    pub replace: String,
    pub case_sensitive: bool,
    /// none | upper | lower | title
    pub case_mode: String,
    /// none | remove | underscore
    pub space_mode: String,
}

impl Default for RenameRules {
    fn default() -> Self {
        Self {
            template: "{name}".into(),
            num_start: 1,
            num_step: 1,
            date_format: "yyyyMMdd".into(),
            find: String::new(),
            replace: String::new(),
            case_sensitive: false,
            case_mode: "none".into(),
            space_mode: "none".into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePreviewEntry {
    pub path: String,
    pub dir: String,
    pub old_name: String,
    pub new_name: String,
    pub error: Option<String>,
    pub conflict: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePreview {
    pub entries: Vec<RenamePreviewEntry>,
    pub has_conflicts: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameResult {
    pub path: String,
    pub old_name: String,
    pub new_name: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

/// (路径, 目录, 旧名, 新名, 错误)
type Computed = (String, String, String, String, Option<String>);

fn is_invalid_win_char(c: char) -> bool {
    matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// Windows 保留设备名（CON、PRN、AUX、NUL、COM1-9、LPT1-9，含任意扩展名形式）
fn is_reserved_win_name(stem: &str) -> bool {
    let trimmed = stem.trim_end_matches([' ', '.']);
    let base = trimmed.split('.').next().unwrap_or("");
    let up = base.to_ascii_uppercase();
    if matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if up.len() == 4 && (up.starts_with("COM") || up.starts_with("LPT")) {
        if let Some(d) = up.chars().nth(3) {
            return d.is_ascii_digit() && d != '0';
        }
    }
    false
}

fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    // 基于 char 的匹配：大小写折叠只用于比较，切片始终落在 char 边界上，
    // 避免「小写化改变字节长度」（如 İ → i̇）导致的 panic 或错位
    let needle_lower: Vec<char> = needle
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let chars: Vec<char> = haystack.chars().collect();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < chars.len() {
        if i + needle_lower.len() <= chars.len() {
            let mut matched = true;
            for j in 0..needle_lower.len() {
                let hc = chars[i + j].to_lowercase().next().unwrap_or(chars[i + j]);
                if hc != needle_lower[j] {
                    matched = false;
                    break;
                }
            }
            if matched {
                out.push_str(replacement);
                i += needle_lower.len();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if cap_next {
                out.extend(c.to_uppercase());
                cap_next = false;
            } else {
                out.extend(c.to_lowercase());
            }
        } else {
            out.push(c);
            cap_next = true;
        }
    }
    out
}

/// 本地日期时间组件（用于 {date} 占位符）
#[derive(Clone, Copy)]
struct DateParts {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

#[cfg(windows)]
fn local_date_parts() -> DateParts {
    #[repr(C)]
    struct SysTime {
        w_year: u16,
        w_month: u16,
        _w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        _w_milliseconds: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(lp_system_time: *mut SysTime);
    }
    let mut st = SysTime {
        w_year: 2026,
        w_month: 1,
        _w_day_of_week: 0,
        w_day: 1,
        w_hour: 0,
        w_minute: 0,
        w_second: 0,
        _w_milliseconds: 0,
    };
    // SAFETY: st 是合法指针，GetLocalTime 只向其中写入系统本地时间
    unsafe { GetLocalTime(&mut st) };
    DateParts {
        year: st.w_year as i64,
        month: st.w_month as u32,
        day: st.w_day as u32,
        hour: st.w_hour as u32,
        minute: st.w_minute as u32,
        second: st.w_second as u32,
    }
}

#[cfg(not(windows))]
fn local_date_parts() -> DateParts {
    // 回退：UTC 日期（非 Windows 平台不换算本地时区）
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let tod = secs.rem_euclid(86400);
    DateParts {
        year: y,
        month: m,
        day: d,
        hour: (tod / 3600) as u32,
        minute: ((tod % 3600) / 60) as u32,
        second: (tod % 60) as u32,
    }
}

#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 按格式串输出日期时间：支持 yyyy/yy、MM、dd、HH、mm、ss，其余字符原样保留
fn format_date(d: &DateParts, fmt: &str) -> String {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut run = 1;
        while i + run < chars.len() && chars[i + run] == c {
            run += 1;
        }
        match c {
            'y' => {
                let y4 = format!("{:04}", d.year);
                if run >= 4 {
                    out.push_str(&y4);
                } else {
                    out.push_str(&y4[y4.len() - 2..]);
                }
            }
            'M' => out.push_str(&format!("{:0width$}", d.month, width = run.min(2))),
            'd' => out.push_str(&format!("{:0width$}", d.day, width = run.min(2))),
            'H' => out.push_str(&format!("{:0width$}", d.hour, width = run.min(2))),
            'm' => out.push_str(&format!("{:0width$}", d.minute, width = run.min(2))),
            's' => out.push_str(&format!("{:0width$}", d.second, width = run.min(2))),
            _ => out.push(c),
        }
        i += run;
    }
    out
}

/// 渲染文件名模板：{name} {upper} {lower} {num} {num:3} {date} {date:yyyy-MM-dd}
/// 未知占位符与未闭合的 { 原样保留，交给用户从预览中发现
fn render_template(template: &str, name: &str, num: i64, date: &DateParts, default_date_fmt: &str) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::with_capacity(template.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '{' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && chars[j] != '}' {
            j += 1;
        }
        if j >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let inner: String = chars[i + 1..j].iter().collect();
        let (tok, param) = match inner.split_once(':') {
            Some((t, p)) => (t, Some(p)),
            None => (inner.as_str(), None),
        };
        match tok {
            "name" => out.push_str(name),
            "upper" => out.push_str(&name.to_uppercase()),
            "lower" => out.push_str(&name.to_lowercase()),
            "num" => {
                let width = param
                    .and_then(|p| p.parse::<usize>().ok())
                    .unwrap_or(0)
                    .min(10);
                out.push_str(&format!("{:0width$}", num, width = width));
            }
            "date" => out.push_str(&format_date(date, param.unwrap_or(default_date_fmt))),
            _ => {
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
        }
        i = j + 1;
    }
    out
}

/// 模板 → 查找替换 → 空格处理 → 大小写转换
fn compute_new_stem(stem: &str, rules: &RenameRules, num: i64, date: &DateParts) -> String {
    let template = if rules.template.trim().is_empty() {
        "{name}"
    } else {
        &rules.template
    };
    let mut s = render_template(template, stem, num, date, &rules.date_format);
    if !rules.find.is_empty() {
        if rules.case_sensitive {
            s = s.replace(&rules.find, &rules.replace);
        } else {
            s = replace_case_insensitive(&s, &rules.find, &rules.replace);
        }
    }
    match rules.space_mode.as_str() {
        "remove" => s.retain(|c| !c.is_whitespace()),
        "underscore" => s = s.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect(),
        _ => {}
    }
    match rules.case_mode.as_str() {
        "upper" => s = s.to_uppercase(),
        "lower" => s = s.to_lowercase(),
        "title" => s = title_case(&s),
        _ => {}
    }
    s
}

fn compute_new_names(paths: &[String], rules: &RenameRules) -> Vec<Computed> {
    let date = local_date_parts();
    let mut out = Vec::with_capacity(paths.len());
    let mut counter = rules.num_start;
    for p in paths {
        let path = Path::new(p);
        let dir = path
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let old = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // 分离文件名主干与扩展名（保留扩展名原始大小写；大小写转换只作用于主干）
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = path
            .with_extension("")
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| old.clone());
        let ext_part = if ext.is_empty() {
            String::new()
        } else {
            format!(".{ext}")
        };

        let new_stem = compute_new_stem(&stem, rules, counter, &date);
        counter = counter.saturating_add(rules.num_step);
        let new_name = format!("{}{}", new_stem, ext_part);

        let error = if new_stem.trim().is_empty() {
            Some("@err_empty_name".to_string())
        } else if new_stem.chars().any(is_invalid_win_char)
            || new_stem.chars().any(|c| (c as u32) < 0x20)
        {
            Some("@err_invalid_chars".to_string())
        } else if new_stem.ends_with([' ', '.']) || is_reserved_win_name(&new_stem) {
            Some("@err_reserved_name".to_string())
        } else {
            None
        };
        out.push((p.clone(), dir, old, new_name, error));
    }
    out
}

/// 预览核心逻辑（纯函数，便于测试）
fn preview_rename_core(paths: &[String], rules: &RenameRules) -> RenamePreview {
    let computed = compute_new_names(paths, rules);
    // 批内源文件集合：lower 全路径 -> 是否会在本批中被改名（让出原名）
    let mut sources: HashMap<String, bool> = HashMap::new();
    for (p, _dir, old, new_name, err) in &computed {
        if err.is_none() {
            sources.insert(p.to_lowercase(), !old.eq_ignore_ascii_case(new_name));
        }
    }
    // 同一目录下目标名重复计数
    let mut counts: HashMap<String, u32> = HashMap::new();
    for (_, dir, _, new_name, err) in &computed {
        if err.is_none() {
            let key = format!("{}\\{}", dir.to_lowercase(), new_name.to_lowercase());
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    let mut entries = Vec::with_capacity(computed.len());
    let mut has_conflicts = false;
    for (p, dir, old, new_name, err) in computed {
        let key = format!("{}\\{}", dir.to_lowercase(), new_name.to_lowercase());
        let same_name = old.eq_ignore_ascii_case(&new_name);
        let dup = counts.get(&key).copied().unwrap_or(0) > 1;
        // 磁盘冲突判定：目标是批内「将被改名让位」的源文件时不视为冲突（支持链条重命名 A→B、B→C）
        let exists = if err.is_none() && !same_name {
            match sources.get(&key) {
                Some(true) => false,
                Some(false) => true,
                None => Path::new(&dir).join(&new_name).exists(),
            }
        } else {
            false
        };
        let conflict = dup || exists;
        has_conflicts |= conflict;
        entries.push(RenamePreviewEntry {
            path: p,
            dir,
            old_name: old,
            new_name,
            error: err,
            conflict,
        });
    }
    RenamePreview { entries, has_conflicts }
}

/// 实时预览重命名结果（不修改任何文件）
#[tauri::command]
pub async fn preview_rename(
    paths: Vec<String>,
    rules: RenameRules,
) -> Result<RenamePreview, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(preview_rename_core(&paths, &rules)))
        .await
        .map_err(|e| e.to_string())?
}

/// 执行重命名。支持链条重命名（A→B、B→C 自动排序），并可自动解决重名冲突。
#[tauri::command]
pub async fn apply_rename(
    paths: Vec<String>,
    rules: RenameRules,
    resolve_conflicts: bool,
) -> Result<Vec<RenameResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let computed = compute_new_names(&paths, &rules);
        let mut results: Vec<Option<RenameResult>> = Vec::with_capacity(computed.len());
        results.resize_with(computed.len(), || None);
        let mut pending: Vec<usize> = Vec::new();
        // 批量内所有源文件的完整路径（小写），用于链条重命名的依赖判断
        let mut orig_set: HashSet<String> = HashSet::new();
        for (p, _, _, _, err) in &computed {
            if err.is_none() {
                orig_set.insert(p.to_lowercase());
            }
        }

        for (i, (p, _dir, old, new_name, err)) in computed.iter().enumerate() {
            match err {
                Some(e) => results[i] = Some(RenameResult {
                    path: p.clone(),
                    old_name: old.clone(),
                    new_name: None,
                    ok: false,
                    error: Some(e.clone()),
                }),
                None => {
                    if old.eq_ignore_ascii_case(new_name) {
                        // 名字没变：无需操作，但占用目标名（不放入 orig 移除逻辑）
                        results[i] = Some(RenameResult {
                            path: p.clone(),
                            old_name: old.clone(),
                            new_name: Some(new_name.clone()),
                            ok: true,
                            error: None,
                        });
                    } else {
                        pending.push(i);
                    }
                }
            }
        }

        // 已占用的目标路径（小写）。先登记「名字未变化」的条目。
        let mut used: HashSet<String> = HashSet::new();
        for (i, item) in computed.iter().enumerate() {
            let is_noop_ok = match results.get(i) {
                Some(Some(r)) => r.ok && r.new_name.as_deref() == Some(item.2.as_str()),
                _ => false,
            };
            if is_noop_ok {
                used.insert(format!("{}\\{}", item.1.to_lowercase(), item.2.to_lowercase()));
            }
        }

        loop {
            let mut next_pending: Vec<usize> = Vec::new();
            let mut progressed = false;
            for &i in &pending {
                let (path, dir, _old, new_name, _) = &computed[i];
                let base_key = format!("{}\\{}", dir.to_lowercase(), new_name.to_lowercase());
                // 目标名是否还被某个尚未移动的源文件占用（链条依赖）
                let blocked_by_source = orig_set.contains(&base_key);
                if blocked_by_source || used.contains(&base_key) {
                    next_pending.push(i);
                    continue;
                }
                let target = Path::new(dir).join(new_name);
                let mut final_target = target.clone();
                let mut final_name = new_name.clone();
                if target.exists() {
                    if !resolve_conflicts {
                        results[i] = Some(RenameResult {
                            path: path.clone(),
                            old_name: computed[i].2.clone(),
                            new_name: None,
                            ok: false,
                            error: Some("@err_conflict_exists".to_string()),
                        });
                        progressed = true;
                        continue;
                    }
                    // 自动加序号
                    let parent = target.parent().map(|p| p.to_path_buf());
                    let stem = target
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    let ext = target
                        .extension()
                        .map(|s| format!(".{}", s.to_string_lossy()))
                        .unwrap_or_default();
                    let mut k = 1u32;
                    loop {
                        let candidate = format!("{stem} ({k}){ext}");
                        let cp = parent.as_deref().unwrap_or(Path::new(".")).join(&candidate);
                        let ck = cp.to_string_lossy().to_lowercase();
                        if !cp.exists() && !used.contains(&ck) && !orig_set.contains(&ck) {
                            final_target = cp;
                            final_name = candidate;
                            break;
                        }
                        k += 1;
                    }
                }
                match std::fs::rename(path, &final_target) {
                    Ok(()) => {
                        let fk = final_target.to_string_lossy().to_lowercase();
                        used.insert(fk);
                        orig_set.remove(&path.to_lowercase());
                        results[i] = Some(RenameResult {
                            path: path.clone(),
                            old_name: computed[i].2.clone(),
                            new_name: Some(final_name),
                            ok: true,
                            error: None,
                        });
                        progressed = true;
                    }
                    Err(e) => {
                        results[i] = Some(RenameResult {
                            path: path.clone(),
                            old_name: computed[i].2.clone(),
                            new_name: None,
                            ok: false,
                            error: Some(format!("@err_rename_failed|{e}")),
                        });
                        progressed = true;
                    }
                }
            }
            if next_pending.is_empty() {
                break;
            }
            if !progressed {
                // 卡住：目标名被占用或存在循环重命名（如 A→B、B→A）
                for &i in &next_pending {
                    if !resolve_conflicts {
                        results[i] = Some(RenameResult {
                            path: computed[i].0.clone(),
                            old_name: computed[i].2.clone(),
                            new_name: None,
                            ok: false,
                            error: Some("@err_conflict_occupied".to_string()),
                        });
                        continue;
                    }
                    // 自动改名解决
                    let (path, dir, _old, new_name, _) = &computed[i];
                    let target = Path::new(dir).join(new_name);
                    let parent = target.parent().map(|p| p.to_path_buf());
                    let stem = target
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    let ext = target
                        .extension()
                        .map(|s| format!(".{}", s.to_string_lossy()))
                        .unwrap_or_default();
                    let mut k = 1u32;
                    let mut done = false;
                    while !done {
                        let candidate = format!("{stem} ({k}){ext}");
                        let cp = parent.as_deref().unwrap_or(Path::new(".")).join(&candidate);
                        let ck = cp.to_string_lossy().to_lowercase();
                        if !cp.exists() && !used.contains(&ck) && !orig_set.contains(&ck) {
                            match std::fs::rename(path, &cp) {
                                Ok(()) => {
                                    used.insert(ck);
                                    orig_set.remove(&path.to_lowercase());
                                    results[i] = Some(RenameResult {
                                        path: path.clone(),
                                        old_name: computed[i].2.clone(),
                                        new_name: Some(candidate),
                                        ok: true,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    results[i] = Some(RenameResult {
                                        path: path.clone(),
                                        old_name: computed[i].2.clone(),
                                        new_name: None,
                                        ok: false,
                                        error: Some(format!("@err_rename_failed|{e}")),
                                    });
                                }
                            }
                            done = true;
                        }
                        k += 1;
                    }
                }
                break;
            }
            pending = next_pending;
        }

        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_date() -> DateParts {
        DateParts {
            year: 2026,
            month: 9,
            day: 8,
            hour: 19,
            minute: 5,
            second: 3,
        }
    }

    #[test]
    fn template_tokens_render() {
        let d = test_date();
        assert_eq!(render_template("{name}", "report", 7, &d, "yyyyMMdd"), "report");
        assert_eq!(render_template("{upper}_{num:3}", "abc", 7, &d, "yyyyMMdd"), "ABC_007");
        assert_eq!(render_template("{lower}", "AbC", 1, &d, "yyyyMMdd"), "abc");
        assert_eq!(render_template("{num}_{num:2}", "a", 5, &d, "yyyyMMdd"), "5_05");
        assert_eq!(render_template("{date}_{name}", "abc", 1, &d, "yyyyMMdd"), "20260908_abc");
        assert_eq!(render_template("{date:yyyy-MM-dd}", "a", 1, &d, "yyyyMMdd"), "2026-09-08");
        assert_eq!(
            render_template("{date:yyyyMMdd_HHmmss}", "a", 1, &d, "yyyyMMdd"),
            "20260908_190503"
        );
        // 未知占位符与未闭合的 { 原样保留
        assert_eq!(render_template("keep{unknown}and{name}", "a", 1, &d, "yyyyMMdd"), "keep{unknown}anda");
        assert_eq!(render_template("open{name", "a", 1, &d, "yyyyMMdd"), "open{name");
        // 模板里的其他字符原样保留
        assert_eq!(render_template("IMG_{name}_v2", "pic", 1, &d, "yyyyMMdd"), "IMG_pic_v2");
    }

    #[test]
    fn stem_modifiers_stack() {
        let d = test_date();
        let mut r = RenameRules::default();
        assert_eq!(compute_new_stem("report", &r, 1, &d), "report");
        // 模板 + 序号补零
        r.template = "{name}_{num:2}".into();
        assert_eq!(compute_new_stem("report", &r, 3, &d), "report_03");
        // 模板之后应用查找替换
        r.find = "e".into();
        r.replace = "3".into();
        assert_eq!(compute_new_stem("report", &r, 3, &d), "r3port_03");
        // 空模板回退为原名
        let mut r0 = RenameRules::default();
        r0.template = "  ".into();
        assert_eq!(compute_new_stem("abc", &r0, 1, &d), "abc");
        // 空格处理
        let mut r2 = RenameRules::default();
        r2.space_mode = "underscore".into();
        assert_eq!(compute_new_stem("hello world", &r2, 1, &d), "hello_world");
        r2.space_mode = "remove".into();
        assert_eq!(compute_new_stem("hello world", &r2, 1, &d), "helloworld");
        // 大小写转换（作用于最终结果）
        let mut r3 = RenameRules::default();
        r3.case_mode = "upper".into();
        assert_eq!(compute_new_stem("abc", &r3, 1, &d), "ABC");
        let mut r4 = RenameRules::default();
        r4.case_mode = "title".into();
        assert_eq!(compute_new_stem("hello world", &r4, 1, &d), "Hello World");
    }

    #[test]
    fn compute_names_template_sequence_and_ext() {
        let paths = vec![
            "C:\\dir\\b.txt".to_string(),
            "C:\\dir\\a.txt".to_string(),
        ];
        let mut r = RenameRules::default();
        r.template = "{num:3}_{name}".into();
        let out = compute_new_names(&paths, &r);
        assert_eq!(out[0].3, "001_b.txt");
        assert_eq!(out[1].3, "002_a.txt");
        // 扩展名保持不变
        assert!(out[0].3.ends_with(".txt"));
        // 步长与起始值
        let mut r2 = RenameRules::default();
        r2.template = "{num}_{name}".into();
        r2.num_start = 10;
        r2.num_step = 5;
        let out2 = compute_new_names(&paths, &r2);
        assert_eq!(out2[0].3, "10_b.txt");
        assert_eq!(out2[1].3, "15_a.txt");
    }

    #[test]
    fn invalid_chars_detected() {
        let paths = vec!["C:\\dir\\ok.txt".to_string()];
        let mut r = RenameRules::default();
        r.template = "a:b{name}".into();
        let out = compute_new_names(&paths, &r);
        assert!(out[0].4.is_some());
    }

    #[test]
    fn compute_names_multibyte_stem_and_special_ext() {
        let r = RenameRules::default();
        // 多字节主干 + 普通扩展名
        let out = compute_new_names(&["C:\\dir\\报表.txt".to_string()], &r);
        assert_eq!(out[0].3, "报表.txt");
        // 扩展名小写化后字节数会变化的字符（如 İ），此前会在字节切片处 panic
        let out2 = compute_new_names(&["C:\\dir\\文件.İ".to_string()], &r);
        assert_eq!(out2[0].3, "文件.İ");
        // 无扩展名文件
        let out3 = compute_new_names(&["C:\\dir\\README".to_string()], &r);
        assert_eq!(out3[0].3, "README");
        // 扩展名保持原始大小写
        let out4 = compute_new_names(&["C:\\dir\\A.TXT".to_string()], &r);
        assert_eq!(out4[0].3, "A.TXT");
    }

    #[test]
    fn replace_case_insensitive_multibyte_safe() {
        // 大小写折叠改变字节长度的字符（İ → i̇）不得 panic、不得错位
        assert_eq!(replace_case_insensitive("İx", "x", "y"), "İy");
        assert_eq!(replace_case_insensitive("AbcAbc", "abc", "-"), "--");
        assert_eq!(replace_case_insensitive("报表", "表", "单"), "报单");
        assert_eq!(replace_case_insensitive("Aİ", "i", "X"), "AX");
    }

    #[test]
    fn reserved_names_detected() {
        let r = RenameRules::default();
        for name in ["CON.txt", "NUL", "COM1.log", "LPT9", "PRN"] {
            let out = compute_new_names(&[format!("C:\\dir\\{name}")], &r);
            assert!(out[0].4.is_some(), "应拒绝保留名：{name}");
        }
        // 结尾点/空格也会被 Windows 剥离，应拒绝
        let out = compute_new_names(&["C:\\dir\\x..".to_string()], &r);
        assert!(out[0].4.is_some());
        // 普通名字不受影响
        let out2 = compute_new_names(&["C:\\dir\\COM10.txt".to_string()], &r);
        assert!(out2[0].4.is_none());
    }
}
