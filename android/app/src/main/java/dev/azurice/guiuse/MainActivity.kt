package dev.azurice.guiuse

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

class MainActivity : Activity() {

    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        status = TextView(this).apply {
            textSize = 14f
            setPadding(24, 24, 24, 24)
            setTextColor(Color.DKGRAY)
        }

        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(32, 64, 32, 32)
            addView(status)
            addView(button("刷新状态") { renderStatus() })
            addView(button("无障碍设置") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) })
            addView(
                button("通知访问设置") {
                    startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                },
            )
        }

        setContentView(ScrollView(this).apply { addView(column) })
        renderStatus()
    }

    override fun onResume() {
        super.onResume()
        if (::status.isInitialized) renderStatus()
    }

    private fun button(label: String, onClick: () -> Unit) =
        Button(this).apply {
            text = label
            setOnClickListener { onClick() }
        }

    private fun renderStatus() {
        status.text = buildString {
            appendLine("无障碍服务：${if (isAccessibilityEnabled()) "已启用" else "未启用"}")
            appendLine("通知访问：${if (isNotificationListenerEnabled()) "已启用" else "未启用"}")
            appendLine()
            append("下一步：连接 host 的 WebSocket，把界面树与通知事件推出去。")
        }
    }

    private fun isAccessibilityEnabled(): Boolean = isComponentEnabled(
        "enabled_accessibility_services",
        DeviceService::class.java,
    )

    private fun isNotificationListenerEnabled(): Boolean = isComponentEnabled(
        "enabled_notification_listeners",
        NotificationService::class.java,
    )

    private fun isComponentEnabled(setting: String, service: Class<*>): Boolean {
        val expected = ComponentName(this, service).flattenToString()
        val enabled = Settings.Secure.getString(contentResolver, setting) ?: return false
        return enabled.split(':').any { it.equals(expected, ignoreCase = true) }
    }
}
