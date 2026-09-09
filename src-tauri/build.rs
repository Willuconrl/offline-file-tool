fn main() {
    // 让 cargo 感知前端资源变化：ui/ 下任何文件改动都会触发重新嵌入与编译
    println!("cargo:rerun-if-changed=../ui");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    tauri_build::build()
}
