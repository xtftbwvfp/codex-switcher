/**
 * 中转站 / OpenAI 兼容服务的预设列表。
 *
 * 选预设后会自动填 base_url + usage_preset，用户只需贴 sk-key。
 *
 * `usage_preset` 字段命中后端 Rust 内置 fetcher 名（见 `usage.rs`）：
 *   - "openai_compat": GET {base}/v1/usage with Bearer
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
     * - "chat_completions" —— 上游只懂 /chat/completions（GLM Coding Plan / 通用 OpenAI Chat），proxy 翻译
     */
    relay_protocol?: 'responses' | 'chat_completions';
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
    },
    {
        id: 'custom',
        name: '自定义中转站',
        base_url: '',
        usage_preset: null,
        description: '手动填 base_url，自选 usage 策略',
    },
];
