/**
 * 中转站 / OpenAI 兼容服务的预设列表。
 *
 * 选预设后会自动填 base_url + usage_preset，用户只需贴 API Key。
 *
 * `usage_preset` 字段命中后端 Rust 内置 fetcher 名（见 `usage.rs`）：
 *   - "openai_compat": GET {base}/v1/usage with Bearer
 *   - "mimo_token_plan": MiMo 控制台 Cookie → /api/v1/tokenPlan/usage
 *   - null: 不拉余额（中转站没标准 usage 接口时用）
 *
 * 加新条目时：除非中转站确实暴露 OpenAI 兼容的 /v1/usage，否则 usage_preset 用 null。
 */
export interface RelayPreset {
    /** 内部唯一标识（不展示） */
    id: string;
    /** 默认账号名（用户可改） */
    name: string;
    /** 必填，OpenAI 兼容 base_url，不带尾斜杠 */
    base_url: string;
    /** 中转站主页（用户参考，可选） */
    homepage?: string;
    /** 后端内置 usage fetcher preset 名；null = 不拉余额 */
    usage_preset?: string | null;
    /** UI 显示用的一行说明 */
    description?: string;
    /** 模型映射兜底（codex 端发的所有未命中 map 的 model 都替换成它） */
    model_fallback?: string | null;
    /** 模型映射表（key=客户端 model，value=中转站 model） */
    model_map?: Record<string, string> | null;
    /**
     * 上游协议 wire format：
     * - "responses"（默认 / 不填 = 等价）—— 上游原生支持 codex /v1/responses（Unity2、ChatGPT 子集、OpenAI key）
     * - "chat_completions" —— 上游只懂 /chat/completions（GLM/MiMo Coding Plan / 通用 OpenAI Chat），proxy 翻译
     */
    relay_protocol?: 'responses' | 'chat_completions';
    /** 展示用：1–3 字符 monogram（卡片左侧图标的文字） */
    mark?: string;
    /** 展示用：monogram 背景色（hex） */
    color?: string;
    /** 展示用：在 AddRelay 卡片选择器中的分组 */
    group?: '通用中转' | 'CODING PLAN' | '三方模型' | '自定义';
    /** 默认 API Key 前缀（占位提示用） */
    auth_prefix?: string;
    /**
     * 业务分类（用于 UI 过滤胶囊 + 行内标签）：
     * - `aggregator` —— 聚合中转（new-api/sub2api/CLIProxyAPI 一类的 reseller，PinCC/Unity2/FreeModel/PackyCode 等）
     * - `coding_plan` —— 厂商自家的 Coding Plan / Token Plan 订阅（GLM Coding Plan / MiMo Token Plan / 火山 Coding Plan 等）
     * - `third_party` —— 厂商按量付费 API（DeepSeek / Kimi / OpenRouter / Fireworks 等）
     */
    category?: 'aggregator' | 'coding_plan' | 'third_party';
}

export const RELAY_PRESETS: RelayPreset[] = [
    {
        id: 'glm',
        name: '智谱 GLM',
        base_url: 'https://open.bigmodel.cn/api/paas/v4',
        homepage: 'https://docs.bigmodel.cn/cn/guide/develop/openai/introduction',
        // GLM 余额走自家 monitor 接口（不是 OpenAI /v1/usage）
        usage_preset: 'glm_zhipu',
        // codex 端发的 gpt-5.5 / gpt-4o 等 GLM 不认识，统一映射成 glm-5.1
        // 用户可在表单里覆盖（如 gpt-4o-mini → glm-5.1-x）
        model_fallback: 'glm-5.1',
        model_map: {
            'gpt-5.5': 'glm-5.1',
            'gpt-5': 'glm-5.1',
            'gpt-5-codex': 'glm-5.1',
            'gpt-4o': 'glm-5',
            'gpt-4o-mini': 'glm-5.1-x',
            'o1': 'glm-5.1',
            'o1-mini': 'glm-5.1-x',
        },
        description: 'GLM-5.1，OpenAI 兼容；模型自动映射 gpt-* → glm-*',
        mark: 'GLM', color: '#4F46E5', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'glm_coding',
        name: 'GLM Coding Plan',
        // GLM Coding 套餐专属端点（与普通 paas/v4 不同）；只暴露 /chat/completions
        base_url: 'https://open.bigmodel.cn/api/coding/paas/v4',
        homepage: 'https://docs.bigmodel.cn/cn/guide/start/coding-plan',
        usage_preset: 'glm_zhipu',
        relay_protocol: 'chat_completions',
        model_fallback: 'glm-5.1',
        model_map: {
            'gpt-5.5': 'glm-5.1',
            'gpt-5': 'glm-5.1',
            'gpt-5-codex': 'glm-5.1',
            'gpt-4o': 'glm-5',
            'gpt-4o-mini': 'glm-5.1-x',
            'o1': 'glm-5.1',
            'o1-mini': 'glm-5.1-x',
        },
        description: 'GLM Coding Plan（codex /v1/responses ↔ /chat/completions 翻译，内置）',
        mark: 'GLM', color: '#4F46E5', group: 'CODING PLAN', auth_prefix: 'sk-',
        category: 'coding_plan',
    },
    {
        id: 'mimo_token_plan_sgp',
        name: 'Xiaomi MiMo Token Plan',
        // MiMo Token Plan 专属端点；官方文档说明 MiMo 暂不适配 Responses API，只适用于 Chat Completions。
        base_url: 'https://token-plan-sgp.xiaomimimo.com/v1',
        homepage: 'https://platform.xiaomimimo.com/console/plan-manage',
        usage_preset: 'mimo_token_plan',
        relay_protocol: 'chat_completions',
        model_fallback: 'mimo-v2.5-pro',
        model_map: {
            'gpt-5.5': 'mimo-v2.5-pro',
            'gpt-5': 'mimo-v2.5-pro',
            'gpt-5-codex': 'mimo-v2.5-pro',
            'gpt-4o': 'mimo-v2.5-pro',
            'gpt-4o-mini': 'mimo-v2.5-pro',
            'o1': 'mimo-v2.5-pro',
            'o1-mini': 'mimo-v2.5-pro',
        },
        description: 'Xiaomi MiMo-V2.5 Token Plan（tp-key；配额用控制台 Cookie）',
        mark: 'Mi', color: '#FF6900', group: 'CODING PLAN', auth_prefix: 'tp-',
        category: 'coding_plan',
    },
    // ────────────────────────────────────────────────────────────────
    // A 类：通用 Responses 中转 —— 基于 new-api / sub2api / CLIProxyAPI
    // 的第三方 codex 中转都用这条。用户自己填 base_url + Bearer key。
    // ────────────────────────────────────────────────────────────────
    {
        id: 'generic_responses_relay',
        name: '通用 Responses 中转（new-api / CLIProxyAPI / sub2api）',
        base_url: '',
        // "auto" → 后端 probe_relay_usage_preset 自动探测：
        //   new-api → /v1/dashboard/billing/* ；sub2api → /v1/usage ；CLIProxyAPI → 不拉取
        usage_preset: 'auto',
        relay_protocol: 'responses',
        model_fallback: 'gpt-5.5',
        description: 'PinCC / PackyCode / AICodeMirror / 自建 CLIProxyAPI 等；余额自动探测',
        mark: '⇄', color: '#64748B', group: '通用中转', auth_prefix: 'sk-',
        category: 'aggregator',
    },
    // ────────────────────────────────────────────────────────────────
    // B 类：厂商 Coding Plan / Token Plan 直连
    // 全部走 chat_completions 翻译。⚠️ 翻译器目前只在 GLM 上完全验证过，
    // 其他厂商可能撞到 reasoning_content / tool_calls 的 quirk，
    // 需要在 relay_translate.rs 里逐个 case-by-case 处理。
    // ────────────────────────────────────────────────────────────────
    {
        id: 'deepseek_api',
        name: 'DeepSeek',
        base_url: 'https://api.deepseek.com/v1',
        homepage: 'https://api-docs.deepseek.com/',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        // 顶端用 V4 Pro（含 thinking）；轻量请求映到 V4 Flash 节省费用。
        // 实测：gpt-5 / gpt-5.5 / gpt-5-codex / o1 走 Pro；gpt-4o / gpt-4o-mini / o1-mini 走 Flash。
        model_fallback: 'deepseek-v4-pro',
        model_map: {
            'gpt-5.5': 'deepseek-v4-pro',
            'gpt-5': 'deepseek-v4-pro',
            'gpt-5-codex': 'deepseek-v4-pro',
            'gpt-4o': 'deepseek-v4-flash',
            'gpt-4o-mini': 'deepseek-v4-flash',
            'o1': 'deepseek-v4-pro',
            'o1-mini': 'deepseek-v4-flash',
        },
        description: 'DeepSeek 按量付费（V4 Pro / V4 Flash）；OpenAI Chat 兼容',
        mark: 'DS', color: '#1E40AF', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'moonshot_kimi',
        name: 'Moonshot Kimi',
        base_url: 'https://api.moonshot.cn/v1',
        homepage: 'https://platform.moonshot.cn/docs',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'kimi-k2-0905-preview',
        model_map: {
            'gpt-5.5': 'kimi-k2-0905-preview',
            'gpt-5': 'kimi-k2-0905-preview',
            'gpt-5-codex': 'kimi-k2-0905-preview',
            'gpt-4o': 'kimi-k2-0905-preview',
            'gpt-4o-mini': 'kimi-k2-0905-preview',
            'o1': 'kimi-k2-0905-preview',
            'o1-mini': 'kimi-k2-0905-preview',
        },
        description: 'Moonshot Kimi K2（按量付费 / 中国订阅 / 国际订阅），OpenAI Chat 兼容',
        mark: 'K', color: '#0F0F10', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'minimax_api',
        name: 'MiniMax',
        base_url: 'https://api.minimax.chat/v1',
        homepage: 'https://platform.minimaxi.com/document/',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'MiniMax-M2',
        model_map: {
            'gpt-5.5': 'MiniMax-M2',
            'gpt-5': 'MiniMax-M2',
            'gpt-5-codex': 'MiniMax-M2',
            'gpt-4o': 'MiniMax-M2',
            'gpt-4o-mini': 'MiniMax-M2',
            'o1': 'MiniMax-M2',
            'o1-mini': 'MiniMax-M2',
        },
        description: 'MiniMax M2（按量付费 / 中国订阅 / 国际订阅），OpenAI Chat 兼容',
        mark: 'MM', color: '#7C3AED', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'alibaba_dashscope',
        name: '阿里 DashScope (通义千问)',
        base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
        homepage: 'https://help.aliyun.com/zh/dashscope/',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'qwen3-max',
        model_map: {
            'gpt-5.5': 'qwen3-max',
            'gpt-5': 'qwen3-max',
            'gpt-5-codex': 'qwen3-coder-plus',
            'gpt-4o': 'qwen-plus',
            'gpt-4o-mini': 'qwen-turbo',
            'o1': 'qwen3-max',
            'o1-mini': 'qwen-plus',
        },
        description: '阿里百炼 / 通义千问，OpenAI 兼容模式；qwen3-max 兜底',
        mark: '通义', color: '#FF6A00', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'volcengine_ark',
        name: '火山方舟 (ByteDance Doubao)',
        base_url: 'https://ark.cn-beijing.volces.com/api/v3',
        homepage: 'https://www.volcengine.com/docs/82379',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        // ⚠️ 火山方舟的 model id 是用户在控制台创建 endpoint 时分配的 ep-xxx，
        // 这里只给一个占位；用户必须改成自己 endpoint 的实际 id。
        model_fallback: 'doubao-seed-1-6-thinking',
        description: '火山方舟 (Coding Plan / Agent Plan / 按量付费)；model 字段需用户填实际 endpoint id',
        mark: '火', color: '#DC2626', group: 'CODING PLAN', auth_prefix: 'sk-',
        category: 'coding_plan',
    },
    {
        id: 'tencent_hunyuan',
        name: '腾讯混元',
        base_url: 'https://api.hunyuan.cloud.tencent.com/v1',
        homepage: 'https://cloud.tencent.com/document/product/1729',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'hunyuan-turbos-latest',
        model_map: {
            'gpt-5.5': 'hunyuan-turbos-latest',
            'gpt-5': 'hunyuan-turbos-latest',
            'gpt-5-codex': 'hunyuan-code',
            'gpt-4o': 'hunyuan-turbos-latest',
            'gpt-4o-mini': 'hunyuan-lite',
            'o1': 'hunyuan-t1-latest',
            'o1-mini': 'hunyuan-t1-latest',
        },
        description: '腾讯混元 (Token Plan / TokenHub 按量)；OpenAI Chat 兼容',
        mark: '混', color: '#0EA5E9', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'baidu_qianfan',
        name: '百度千帆 (ERNIE)',
        base_url: 'https://qianfan.baidubce.com/v2',
        homepage: 'https://cloud.baidu.com/doc/WENXINWORKSHOP/index.html',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'ernie-4.5-turbo-128k',
        model_map: {
            'gpt-5.5': 'ernie-4.5-turbo-128k',
            'gpt-5': 'ernie-4.5-turbo-128k',
            'gpt-5-codex': 'ernie-4.5-turbo-128k',
            'gpt-4o': 'ernie-4.5-turbo-128k',
            'gpt-4o-mini': 'ernie-speed-128k',
            'o1': 'ernie-x1-turbo-32k',
            'o1-mini': 'ernie-x1-turbo-32k',
        },
        description: '百度千帆 / 文心一言 ERNIE（按量付费 / 中国订阅）',
        mark: '千', color: '#3B82F6', group: '三方模型', auth_prefix: 'bce-',
        category: 'third_party',
    },
    {
        id: 'ucloud_modelverse',
        name: '优云智算 UCloud Modelverse',
        base_url: 'https://deepseek.uk-tokyo.ucloud-global.com/v1',
        homepage: 'https://www.ucloud.cn/site/active/modelverse.html',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'glm-4.7',
        description: 'UCloud Modelverse (Coding Plan + 按量付费 国内/海外)；多模型聚合',
        mark: 'U', color: '#0066FF', group: 'CODING PLAN', auth_prefix: 'sk-',
        category: 'coding_plan',
    },
    {
        id: 'fireworks_ai',
        name: 'Fireworks AI',
        base_url: 'https://api.fireworks.ai/inference/v1',
        homepage: 'https://docs.fireworks.ai/',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        // Fireworks 的模型 id 形如 accounts/fireworks/models/glm-4p6
        model_fallback: 'accounts/fireworks/models/glm-4p6',
        model_map: {
            'gpt-5.5': 'accounts/fireworks/models/glm-4p6',
            'gpt-5': 'accounts/fireworks/models/glm-4p6',
            'gpt-5-codex': 'accounts/fireworks/models/qwen3-coder-480b-a35b-instruct',
            'gpt-4o': 'accounts/fireworks/models/llama-v3p3-70b-instruct',
            'gpt-4o-mini': 'accounts/fireworks/models/llama-v3p1-8b-instruct',
        },
        description: 'Fireworks AI 海外高速推理（按量 / Fire Pass 国际订阅）；OpenAI Chat 兼容',
        mark: 'FW', color: '#7C3AED', group: '三方模型', auth_prefix: 'fw-',
        category: 'third_party',
    },
    {
        id: 'stepfun_step',
        name: '阶跃星辰 Stepfun',
        base_url: 'https://api.stepfun.com/v1',
        homepage: 'https://platform.stepfun.com/docs/',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        model_fallback: 'step-3',
        model_map: {
            'gpt-5.5': 'step-3',
            'gpt-5': 'step-3',
            'gpt-5-codex': 'step-3',
            'gpt-4o': 'step-2-16k',
            'gpt-4o-mini': 'step-1-flash',
            'o1': 'step-r-mini',
            'o1-mini': 'step-r-mini',
        },
        description: '阶跃星辰 step 系列 (按量付费 / 中国订阅 / 国际订阅)',
        mark: 'St', color: '#0F766E', group: '三方模型', auth_prefix: 'sk-',
        category: 'third_party',
    },
    {
        id: 'openrouter',
        name: 'OpenRouter (500+ 模型聚合)',
        base_url: 'https://openrouter.ai/api/v1',
        homepage: 'https://openrouter.ai/docs',
        usage_preset: null,
        relay_protocol: 'chat_completions',
        // OpenRouter 用 vendor/model 形式；不设兜底，让用户自己选
        model_fallback: 'openai/gpt-5.5',
        description: 'OpenRouter 聚合 500+ 模型；模型 id 形如 anthropic/claude-sonnet-4.6',
        mark: 'OR', color: '#10B981', group: '三方模型', auth_prefix: 'sk-or-',
        category: 'third_party',
    },
    {
        id: 'custom',
        name: '自定义中转站',
        base_url: '',
        usage_preset: 'auto',
        description: '手动填 base_url；余额自动探测',
        mark: '+', color: '#64748B', group: '自定义', auth_prefix: 'sk-',
        category: 'aggregator',
    },
];
