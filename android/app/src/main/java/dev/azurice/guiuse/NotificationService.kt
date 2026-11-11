package dev.azurice.guiuse

import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log

/**
 * 通知读取与事件推送的入口。
 *
 * 之后在这里实现：
 * - getActiveNotifications() 初始快照
 * - onNotificationPosted / onNotificationRemoved 增量事件
 * - 同一 key 的更新合并，RemoteInput 回复
 */
class NotificationService : NotificationListenerService() {

    override fun onListenerConnected() {
        Log.i(TAG, "notification listener connected: ${activeNotifications?.size ?: 0} active")
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        sbn ?: return
        Log.d(TAG, "posted: ${sbn.packageName}")
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        sbn ?: return
        Log.d(TAG, "removed: ${sbn.packageName}")
    }

    private companion object {
        const val TAG = "GuiUse/NotificationService"
    }
}
