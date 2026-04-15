# Chrome Sync 账号映射算法

## 目标

在不修改 Chromium 源码的前提下，通过 LD_PRELOAD 注入的代理服务，将 Chrome Sync 请求与用户邮箱关联。

## 数据来源

Chrome 的用户数据目录（默认 `~/.config/google-chrome/`）下，每个 Profile 目录中有一个 `Preferences` JSON 文件，包含以下关键字段：

### 1. 账号信息 — `account_info`

```json
"account_info": [
  {
    "email": "user@gmail.com",
    "gaia": "109944815437949063750"
  }
]
```

### 2. Sync 传输数据 — `sync.transport_data_per_account`

按 `gaia_id_hash` 分组，每个账号有独立的 `cache_guid`：

```json
"sync": {
  "transport_data_per_account": {
    "<gaia_id_hash>": {
      "sync.cache_guid": "cGVaHRBTN6hqTw/PjD1XdQ=="
    }
  }
}
```

### 3. gaia_id_hash 计算方式

```
gaia_id_hash = base64_encode(sha256(gaia_id))
```

示例：

```
gaia_id:      "109944815437949063750"
sha256:       5d0ec7f8a11eaf3fe93da37776b2b933aaac43b870cb43422f620fd9ec4797e1
base64:       "XQ7H+KEerz/pPaN3srkzOqxDuHDLQ0LvYg/Z7EeX4ZY="
```

## 映射算法

```
输入: Chrome Sync 请求 URL 中的 client_id 参数
输出: 用户邮箱

步骤:
1. 扫描 user-data-dir 下所有 Profile 目录（Default, Profile 1, Profile 2, ...）
2. 对每个 Profile，读取 Preferences 文件
3. 遍历 account_info 数组，获取所有 (gaia_id, email) 对
4. 对每个 gaia_id 计算 hash: base64(sha256(gaia_id))
5. 用 hash 在 sync.transport_data_per_account 中查找对应的 sync.cache_guid
6. 建立映射表: cache_guid → email

代理收到请求时:
1. 从 URL 解析 client_id 参数（即 cache_guid）
2. 在映射表中查找对应的 email
3. 添加 X-Sync-User-Email header
```

## 请求 URL 格式

Chrome 发出的 Sync 请求 URL 由 Chromium 内部拼接：

```
原始 sync-url:  http://127.0.0.1:PORT/chrome-sync

最终请求 URL:   http://127.0.0.1:PORT/chrome-sync/command/?client=Google+Chrome&client_id=<cache_guid>
```

源码路径：
- `components/sync/engine/sync_manager_impl.cc` — `MakeConnectionURL()` 追加 `/command/` 路径
- `components/sync/engine/net/url_translator.cc` — `AppendSyncQueryString()` 追加 `client` 和 `client_id` 参数

## 多 Profile 场景

Chrome 多 Profile 共享一个主进程（browser process）。每个 Profile 的 Sync 请求使用各自账号的 `cache_guid` 作为 `client_id`，因此代理可通过 `client_id` 区分不同 Profile 的请求。

## 完整示例

```
Profile: Default
  account_info[0].email = "alice@gmail.com"
  account_info[0].gaia  = "109944815437949063750"
  gaia_id_hash           = "XQ7H+KEerz/pPaN3srkzOqxDuHDLQ0LvYg/Z7EeX4ZY="
  cache_guid             = "cGVaHRBTN6hqTw/PjD1XdQ=="

Profile: Profile 1
  account_info[0].email = "bob@gmail.com"
  account_info[0].gaia  = "112978327937825080111"
  gaia_id_hash           = "<sha256 hash base64>"
  cache_guid             = "pWuIq2P6R9lBOM4TA34P4Q=="

映射表:
  cGVaHRBTN6hqTw/PjD1XdQ== → alice@gmail.com
  pWuIq2P6R9lBOM4TA34P4Q== → bob@gmail.com

收到请求:
  POST /chrome-sync/command/?client=Google+Chrome&client_id=cGVaHRBTN6hqTw/PjD1XdQ==
  → 查表得到 alice@gmail.com
  → 添加 X-Sync-User-Email: alice@gmail.com
  → 转发到 https://clients4.google.com/chrome-sync/command/...
```
