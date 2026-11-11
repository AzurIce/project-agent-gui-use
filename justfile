default:
    @just --list

# 编译手机端 debug APK
build-android:
    cd android && ./gradlew :app:assembleDebug

# 安装到已连接的设备
install-android:
    cd android && ./gradlew :app:installDebug

# 编译 host 端
build-host:
    cargo build

# 构建文档站点
docs:
    notist build

# 本地预览文档
serve-docs:
    notist serve

# 从 Rust 协议定义重新生成 Kotlin 类型、方法接口、文档表格与清单
protocol:
    cargo run -q -p gui-use-protocol-codegen
    typeshare crates/gui-use-protocol --lang=kotlin --java-package dev.azurice.guiuse.protocol --output-file android/app/src/main/java/dev/azurice/guiuse/protocol/Types.kt
    typeshare crates/gui-use-protocol --lang=typescript --output-file protocol/gui-use.d.ts

# 检查生成产物是否与 Rust 定义一致
protocol-check:
    just protocol
    test -z "$(git status --porcelain -- android/app/src/main/java/dev/azurice/guiuse/protocol protocol)"

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
