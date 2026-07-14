export type MessageKey =
  | "app.name"
  | "nav.chat"
  | "nav.settings"
  | "nav.theme"
  | "theme.system"
  | "theme.light"
  | "theme.dark"
  | "chat.placeholderTitle"
  | "chat.placeholderBody"
  | "chat.needProviderTitle"
  | "chat.needProviderBody"
  | "chat.goSettings"
  | "chat.needModel"
  | "chat.startSessionTitle"
  | "chat.startSessionBody"
  | "chat.newSession"
  | "chat.emptySession"
  | "chat.composerPlaceholder"
  | "chat.send"
  | "chat.stop"
  | "chat.sending"
  | "chat.stopping"
  | "chat.reasoning"
  | "chat.stopped"
  | "chat.error"
  | "chat.selectProvider"
  | "chat.selectModel"
  | "chat.systemPrompt"
  | "chat.systemPromptHint"
  | "chat.systemPromptSave"
  | "chat.copyCode"
  | "chat.copied"
  | "chat.scrollToBottom"
  | "chat.partialCorrupt"
  | "chat.generating"
  | "chat.outline"
  | "sessions.title"
  | "sessions.empty"
  | "sessions.rename"
  | "sessions.delete"
  | "sessions.deleteTitle"
  | "sessions.deleteBody"
  | "sessions.confirmDelete"
  | "sessions.collapse"
  | "sessions.expand"
  | "sessions.resize"
  | "settings.title"
  | "settings.newProvider"
  | "settings.emptyTitle"
  | "settings.emptyBody"
  | "settings.providerList"
  | "settings.modelCount"
  | "settings.draftLabel"
  | "settings.collapse"
  | "settings.expand"
  | "settings.resize"
  | "settings.form.name"
  | "settings.form.protocol"
  | "settings.form.baseUrl"
  | "settings.form.apiKey"
  | "settings.form.apiKeyHintSaved"
  | "settings.form.apiKeyHintNew"
  | "settings.form.clearKey"
  | "settings.form.showKey"
  | "settings.form.hideKey"
  | "settings.form.save"
  | "settings.form.savedHint"
  | "settings.form.discover"
  | "settings.form.delete"
  | "settings.form.protocolOpenai"
  | "settings.form.protocolOpenaiResponses"
  | "settings.form.protocolAnthropicMessages"
  | "settings.form.protocolGeminiGenerateContent"
  | "settings.form.nameRequired"
  | "settings.form.baseUrlRequired"
  | "settings.form.baseUrlInvalid"
  | "settings.form.saveSuccess"
  | "settings.form.mustSaveBeforeDiscover"
  | "settings.form.deleteConfirmTitle"
  | "settings.form.deleteConfirmBody"
  | "settings.form.unsavedTitle"
  | "settings.form.unsavedBody"
  | "settings.form.saveAndContinue"
  | "settings.form.discard"
  | "settings.form.cancel"
  | "settings.form.confirmDelete"
  | "settings.models.title"
  | "settings.models.retained"
  | "settings.models.search"
  | "settings.models.discoverResults"
  | "settings.models.saveModels"
  | "settings.models.manualAdd"
  | "settings.models.manualTitle"
  | "settings.models.manualId"
  | "settings.models.manualName"
  | "settings.models.displayName"
  | "settings.models.manualSubmit"
  | "settings.models.duplicate"
  | "settings.models.emptyRetained"
  | "settings.models.source.discovery"
  | "settings.models.source.manual"
  | "settings.models.temperature"
  | "settings.models.maxTokens"
  | "settings.models.remove"
  | "settings.models.setDefault"
  | "settings.models.isDefault"
  | "settings.models.discovering"
  | "settings.models.discoverFailed"
  | "settings.default.updated"
  | "common.save"
  | "common.cancel"
  | "common.close"
  | "common.loading"
  | "error.unknown"
  | "error.validation"
  | "error.notFound"
  | "error.conflict"
  | "error.auth"
  | "error.rateLimited"
  | "error.providerUnavailable"
  | "error.timeout"
  | "error.transport"
  | "error.protocol"
  | "error.storage"
  | "error.cancelled";

const zhCN: Record<MessageKey, string> = {
  "app.name": "MPrism",
  "nav.chat": "聊天",
  "nav.settings": "模型服务",
  "nav.theme": "主题",
  "theme.system": "跟随系统",
  "theme.light": "亮色",
  "theme.dark": "暗色",
  "chat.placeholderTitle": "聊天工作台",
  "chat.placeholderBody": "选择会话后开始对话。",
  "chat.needProviderTitle": "先配置模型服务",
  "chat.needProviderBody": "添加服务商并保留至少一个模型后，即可开始聊天。",
  "chat.goSettings": "打开模型服务",
  "chat.needModel": "此服务商还没有可用模型",
  "chat.startSessionTitle": "开始新会话",
  "chat.startSessionBody": "创建一个会话以发送消息。",
  "chat.newSession": "新建会话",
  "chat.emptySession": "输入消息开始对话",
  "chat.composerPlaceholder": "输入消息，Enter 发送，Shift+Enter 换行",
  "chat.send": "发送",
  "chat.stop": "停止",
  "chat.sending": "发送中…",
  "chat.stopping": "停止中…",
  "chat.reasoning": "思考过程",
  "chat.stopped": "已停止",
  "chat.error": "生成失败",
  "chat.selectProvider": "选择服务商",
  "chat.selectModel": "选择模型",
  "chat.systemPrompt": "系统提示词",
  "chat.systemPromptHint": "可选，最多 32000 字",
  "chat.systemPromptSave": "保存提示词",
  "chat.copyCode": "复制",
  "chat.copied": "已复制",
  "chat.scrollToBottom": "回到底部",
  "chat.partialCorrupt": "部分历史消息已损坏并被跳过",
  "chat.generating": "生成中",
  "chat.outline": "问题锚点",
  "sessions.title": "会话",
  "sessions.empty": "暂无会话",
  "sessions.rename": "重命名",
  "sessions.delete": "删除",
  "sessions.deleteTitle": "删除会话？",
  "sessions.deleteBody": "会话将从列表中移除。V1 界面无法恢复。",
  "sessions.confirmDelete": "删除会话",
  "sessions.collapse": "折叠会话列表",
  "sessions.expand": "展开会话列表",
  "sessions.resize": "调整会话列表宽度",
  "settings.title": "模型服务",
  "settings.newProvider": "新建服务商",
  "settings.emptyTitle": "还没有服务商",
  "settings.emptyBody": "新建一个 OpenAI-compatible 服务商，填写 Base URL 与 API Key。",
  "settings.providerList": "服务商",
  "settings.modelCount": "{count} 个模型",
  "settings.draftLabel": "未保存草稿",
  "settings.collapse": "折叠服务商列表",
  "settings.expand": "展开服务商列表",
  "settings.resize": "调整服务商列表宽度",
  "settings.form.name": "名称",
  "settings.form.protocol": "协议",
  "settings.form.baseUrl": "Base URL",
  "settings.form.apiKey": "API Key",
  "settings.form.apiKeyHintSaved": "已保存；留空保持不变",
  "settings.form.apiKeyHintNew": "可留空以连接无需鉴权的本地服务",
  "settings.form.clearKey": "清除 Key",
  "settings.form.showKey": "显示",
  "settings.form.hideKey": "隐藏",
  "settings.form.save": "保存配置",
  "settings.form.savedHint": "已保存",
  "settings.form.discover": "获取模型",
  "settings.form.delete": "删除",
  "settings.form.protocolOpenai": "OpenAI-compatible Chat Completions",
  "settings.form.protocolOpenaiResponses": "OpenAI Responses",
  "settings.form.protocolAnthropicMessages": "Anthropic Messages",
  "settings.form.protocolGeminiGenerateContent": "Gemini generateContent",
  "settings.form.nameRequired": "请填写服务商名称",
  "settings.form.baseUrlRequired": "请填写 Base URL",
  "settings.form.baseUrlInvalid": "Base URL 必须是不含 query/fragment 的 http(s) 地址",
  "settings.form.saveSuccess": "服务商已保存",
  "settings.form.mustSaveBeforeDiscover": "请先保存服务商配置，再获取模型",
  "settings.form.deleteConfirmTitle": "删除服务商？",
  "settings.form.deleteConfirmBody": "删除后无法再选择该服务商，但历史消息中的服务商/模型快照仍会保留。",
  "settings.form.unsavedTitle": "有未保存的更改",
  "settings.form.unsavedBody": "切换前请先保存或放弃当前修改。",
  "settings.form.saveAndContinue": "保存并继续",
  "settings.form.discard": "放弃更改",
  "settings.form.cancel": "取消",
  "settings.form.confirmDelete": "删除服务商",
  "settings.models.title": "模型",
  "settings.models.retained": "已保留模型",
  "settings.models.search": "搜索模型",
  "settings.models.discoverResults": "发现结果",
  "settings.models.saveModels": "保存模型",
  "settings.models.manualAdd": "手工添加",
  "settings.models.manualTitle": "手工添加模型",
  "settings.models.manualId": "模型 ID",
  "settings.models.manualName": "显示名称（可选）",
  "settings.models.displayName": "模型名",
  "settings.models.manualSubmit": "添加",
  "settings.models.duplicate": "该模型 ID 已存在",
  "settings.models.emptyRetained": "还没有保留模型。可获取模型列表或手工添加。",
  "settings.models.source.discovery": "发现",
  "settings.models.source.manual": "手工",
  "settings.models.temperature": "temperature",
  "settings.models.maxTokens": "max_tokens",
  "settings.models.remove": "移除",
  "settings.models.setDefault": "设为默认",
  "settings.models.isDefault": "默认",
  "settings.models.discovering": "正在获取模型…",
  "settings.models.discoverFailed": "获取模型失败，已保留现有模型。",
  "settings.default.updated": "默认服务商与模型已更新",
  "common.save": "保存",
  "common.cancel": "取消",
  "common.close": "关闭",
  "common.loading": "加载中…",
  "error.unknown": "发生未知错误",
  "error.validation": "输入无效",
  "error.notFound": "未找到目标资源",
  "error.conflict": "操作冲突",
  "error.auth": "鉴权失败，请检查 API Key",
  "error.rateLimited": "请求过于频繁，请稍后重试",
  "error.providerUnavailable": "模型服务暂时不可用",
  "error.timeout": "请求超时",
  "error.transport": "无法连接模型服务",
  "error.protocol": "模型服务返回无法解析的响应",
  "error.storage": "本地数据读写失败",
  "error.cancelled": "操作已取消",
};

export function t(key: MessageKey, vars?: Record<string, string | number>): string {
  let text = zhCN[key] ?? key;
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      text = text.replaceAll(`{${name}}`, String(value));
    }
  }
  return text;
}

