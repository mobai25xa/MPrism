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
  | "chat.toolCalls"
  | "chat.toolCallsReadonly"
  | "chat.toolCallUnnamed"
  | "chat.finishReason"
  | "chat.retryAfter"
  | "chat.attachImage"
  | "chat.removeImage"
  | "chat.pendingImages"
  | "chat.imageAttachment"
  | "chat.visionUnsupported"
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
  | "settings.reasoning.title"
  | "settings.reasoning.mode"
  | "settings.reasoning.modeAuto"
  | "settings.reasoning.modeOff"
  | "settings.reasoning.modeOn"
  | "settings.reasoning.effort"
  | "settings.reasoning.effortNone"
  | "settings.reasoning.budget"
  | "settings.reasoning.budgetPlaceholder"
  | "settings.reasoning.requestVsResponse"
  | "settings.reasoning.hintChatCompletions"
  | "settings.reasoning.hintResponses"
  | "settings.reasoning.hintAnthropic"
  | "settings.reasoning.hintGemini"
  | "settings.reasoning.hintGeneric"
  | "settings.reasoning.unsupportedControl"
  | "settings.reasoning.budgetInvalid"
  | "settings.tools.title"
  | "settings.tools.hint"
  | "settings.tools.unsupported"
  | "settings.tools.jsonLabel"
  | "settings.tools.jsonPlaceholder"
  | "settings.tools.toolChoice"
  | "settings.tools.choiceAuto"
  | "settings.tools.choiceNone"
  | "settings.tools.choiceRequired"
  | "settings.tools.choiceNamed"
  | "settings.tools.namedTool"
  | "settings.tools.jsonInvalid"
  | "settings.tools.jsonMustBeArray"
  | "settings.tools.itemInvalid"
  | "settings.tools.nameRequired"
  | "settings.tools.parametersObject"
  | "settings.tools.duplicateName"
  | "settings.tools.namedMissing"
  | "settings.tools.choiceWithoutTools"
  | "settings.tools.applyJson"
  | "settings.auth.title"
  | "settings.auth.advanced"
  | "settings.auth.warning"
  | "settings.auth.extraHeaders"
  | "settings.auth.headerName"
  | "settings.auth.headerValue"
  | "settings.auth.addHeader"
  | "settings.auth.removeHeader"
  | "settings.auth.apiKeyQuery"
  | "settings.auth.apiKeyQueryPlaceholder"
  | "settings.auth.apiKeyQueryHint"
  | "settings.auth.headersUnsupported"
  | "settings.auth.queryUnsupported"
  | "settings.auth.headerNameRequired"
  | "settings.auth.headerCrlf"
  | "settings.auth.queryInvalid"
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
  "chat.toolCalls": "工具调用",
  "chat.toolCallsReadonly": "仅展示，应用不会执行",
  "chat.toolCallUnnamed": "未命名工具",
  "chat.finishReason": "结束原因: {reason}",
  "chat.retryAfter": "建议 {ms}ms 后重试",
  "chat.attachImage": "添加图片",
  "chat.removeImage": "移除图片",
  "chat.pendingImages": "待发送图片",
  "chat.imageAttachment": "图片",
  "chat.visionUnsupported": "当前协议不支持图片输入",
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
  "settings.reasoning.title": "推理强度 / 预算",
  "settings.reasoning.mode": "请求侧推理策略",
  "settings.reasoning.modeAuto": "自动（不传控制）",
  "settings.reasoning.modeOff": "关闭",
  "settings.reasoning.modeOn": "开启",
  "settings.reasoning.effort": "effort",
  "settings.reasoning.effortNone": "不指定",
  "settings.reasoning.budget": "budget_tokens",
  "settings.reasoning.budgetPlaceholder": "可选，正整数",
  "settings.reasoning.requestVsResponse":
    "此处为请求侧推理强度；聊天区「思考过程」为响应侧展示，互不替代。",
  "settings.reasoning.hintChatCompletions":
    "当前协议不支持请求侧推理控制；可改用 OpenAI Responses。响应侧「思考过程」仍可展示。",
  "settings.reasoning.hintResponses":
    "Responses 支持 effort（无 Max）；budget 通常会被忽略。",
  "settings.reasoning.hintAnthropic":
    "Anthropic 常用 budget_tokens（建议 ≥1024）；请注意与 max_tokens 的关系。",
  "settings.reasoning.hintGemini":
    "Gemini 支持 level/budget 路径；动态预算请使用「自动」。",
  "settings.reasoning.hintGeneric": "按协议能力配置请求侧推理策略。",
  "settings.reasoning.unsupportedControl": "当前协议不支持请求侧推理控制",
  "settings.reasoning.budgetInvalid": "budget_tokens 须为正整数或留空",
  "settings.tools.title": "工具定义（透传）",
  "settings.tools.hint":
    "仅随请求发送工具定义并展示模型工具调用，应用不会执行工具、不做多轮循环。",
  "settings.tools.unsupported": "当前协议不支持 tools",
  "settings.tools.jsonLabel": "tools JSON 数组",
  "settings.tools.jsonPlaceholder":
    '[\n  {\n    "name": "get_weather",\n    "description": "查询天气",\n    "parameters": { "type": "object", "properties": {} }\n  }\n]',
  "settings.tools.toolChoice": "tool_choice",
  "settings.tools.choiceAuto": "auto",
  "settings.tools.choiceNone": "none",
  "settings.tools.choiceRequired": "required",
  "settings.tools.choiceNamed": "named",
  "settings.tools.namedTool": "指定工具名",
  "settings.tools.jsonInvalid": "tools JSON 无法解析",
  "settings.tools.jsonMustBeArray": "tools 须为 JSON 数组",
  "settings.tools.itemInvalid": "tools 数组元素无效",
  "settings.tools.nameRequired": "tool name 不能为空",
  "settings.tools.parametersObject": "tool parameters 必须是 JSON object",
  "settings.tools.duplicateName": "tool name 重复: {name}",
  "settings.tools.namedMissing": "tool_choice named 必须引用已声明的 tool",
  "settings.tools.choiceWithoutTools": "未配置 tools 时不能设置 tool_choice",
  "settings.tools.applyJson": "应用 JSON",
  "settings.auth.title": "高级鉴权",
  "settings.auth.advanced": "展开高级鉴权选项",
  "settings.auth.warning":
    "自定义请求头可能包含敏感信息；api_key 放入 query 可能被代理日志记录。请仅在确有需要时配置。",
  "settings.auth.extraHeaders": "额外请求头",
  "settings.auth.headerName": "Header 名",
  "settings.auth.headerValue": "Header 值",
  "settings.auth.addHeader": "添加 Header",
  "settings.auth.removeHeader": "移除",
  "settings.auth.apiKeyQuery": "API Key Query 参数名",
  "settings.auth.apiKeyQueryPlaceholder": "例如 key（留空则不使用 query）",
  "settings.auth.apiKeyQueryHint": "非空时将 API Key 作为该 query 参数发送（空 Key 则不发）",
  "settings.auth.headersUnsupported": "当前协议不支持自定义请求头",
  "settings.auth.queryUnsupported": "当前协议不支持 API Key query 参数",
  "settings.auth.headerNameRequired": "Header 名不能为空",
  "settings.auth.headerCrlf": "Header 名/值不能包含换行",
  "settings.auth.queryInvalid": "api_key_query_param 包含非法字符",
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

