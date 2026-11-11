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

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
