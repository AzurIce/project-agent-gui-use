default:
    @just --list

# 编译手机端 debug APK
build-android:
    cd android && ./gradlew :app:assembleDebug

# 安装到已连接的设备
install-android:
    cd android && ./gradlew :app:installDebug

# 仅操作明确选择的 OPPO 和本地测试界面；退出时恢复测试配置
test-device serial:
    python3 scripts/device_smoke.py --serial {{quote(serial)}}

# 编译 host 端
build-host:
    cargo build --locked -p gui-use-host -j8

# 独立 PTC agent（普通 Android 构建不依赖此执行器）
build-agent:
    cargo build --locked -p gui-use-agent -j8

# 在明确选择的 OPPO 自有测试界面上验证 PTC 编排
test-ptc-device serial:
    python3 scripts/device_smoke.py --serial {{quote(serial)}} --ptc-agent

# 构建文档站点
docs: protocol
    notist build

# 本地预览文档
serve-docs: protocol
    notist serve

# 只用 Cargo 生成 Kotlin、TypeScript、OpenRPC 与文档数据
protocol:
    cargo run -q --locked -p gui-use-protocol-codegen

# 检查生成产物是否与 Rust 定义一致
protocol-check:
    cargo run -q --locked -p gui-use-protocol-codegen -- --check

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -j8 -- -D warnings

test:
    cargo test --workspace -j8 -- --test-threads=4
    python3 -m unittest discover -s scripts -p 'test_*.py'
