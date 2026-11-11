package dev.azurice.guiuse

import android.accessibilityservice.AccessibilityService
import android.util.Log
import android.view.accessibility.AccessibilityEvent

/**
 * 界面读取与操作的入口。
 *
 * 之后在这里实现：
 * - 快照：getRootInActiveWindow() / getWindows() 合成稳定节点树
 * - 动作：performAction() 优先，dispatchGesture() 兜底
 * - 事件：WINDOW_STATE_CHANGED / WINDOW_CONTENT_CHANGED / VIEW_TEXT_CHANGED
 */
class DeviceService : AccessibilityService() {

    override fun onServiceConnected() {
        Log.i(TAG, "accessibility service connected")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        event ?: return
        // TODO: 过滤 + 去抖后转发给 host。
    }

    override fun onInterrupt() = Unit

    private companion object {
        const val TAG = "GuiUse/DeviceService"
    }
}
