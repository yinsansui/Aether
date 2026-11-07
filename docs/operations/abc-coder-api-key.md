# AbcCoder 自动配置 API Key

管理接口 `POST /api/admin/integrations/abc-coder/api-key/ensure` 使用现有管理令牌鉴权，
需要 `admin:users:admin` 权限。此接口信任调用服务提供的身份，仅供受信任的 AbcCoder 后端调用。

请求体包括 `coder_user_id`（非空 UUID）及 `claims`（服务端保存的 OIDC merged claims）。
provider 固定为 `custom_oidc_oa`，复用交互登录相同的属性映射和 OAuth 账号解析路径。
因此同一个 provider subject 会命中同一账号。仍遵循 OAuth 模块开关、provider 启用、
注册开关、默认用户分组及初始钱包规则；邮箱冲突不会自动合并身份。

返回 `aether_user_id`、`api_key_id`、`key_name`、`key`、`created_key`。
`key_name` 固定为 `abc-coder`。响应包含完整密钥并设置 no-store，禁止记录响应体。
已存在启用 Key 时稳定选择最早创建的一个；锁定、过期或无法恢复的 Key 返回冲突。
没有启用的同名 Key 时创建普通用户 Key，继承用户权限和钱包，不授予无限额度。

无需数据库迁移。进程内串行处理保证正常重试复用 Key；多副本首次并发可能生成重复 Key，
后续请求稳定选择同一条。服务调用失败后由 AbcCoder 的下次登录或凭据页重试兜底。

AbcCoder 部署设置 `ABC_AETHER_URL` 与 `ABC_AETHER_MANAGEMENT_TOKEN`。
上线前核对双方 subject 命名空间相同；不能用邮箱替代 OAuth subject。
