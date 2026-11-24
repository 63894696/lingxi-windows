# 灵犀拼音输入法 for Windows — 关于

灵犀拼音输入法（Windows）由 BabelSpan · Prisir（湃睿思）提供。

产品定位：面向个人与专业场景的本地中文输入工具。核心能力包括拼音/混拼候选、整句解码、简繁输出、本地词库与用户词典管理，以及可选的语音听写插件。输入链路在本机完成，不依赖账号体系，也不以云端联想为默认能力。

若需借助 AI Agent 按个人习惯定制输入法（词库、模式、查询等），请参阅「开放 API」：默认关闭，按需开启后仅对本机回环地址开放。

官网与下载：https://www.babelspan.com/

版权所有 ? 2026 BabelSpan。本软件依据 GNU Affero General Public License v3.0 or later（AGPL-3.0-or-later）授权，按「原样」提供，不附带任何担保或条件。完整授权见安装目录 LICENSE.txt（AGPL-3.0-or-later）。

## 隐私政策

灵犀拼音采用严格的隐私保护设计，完全离线无账号运行：

- 键入内容、候选结果与词库数据保存在本机，不向第三方上传输入正文。
- 不提供账号登录，不做行为画像，不出售或交换个人输入数据。
- 用户可通过词库管理、黑白名单与学习记录自主维护本地词典。
- 可选的语音听写插件在本机完成识别，不外发音频。
- 开放 API 默认关闭；开启后也仅监听本机 127.0.0.1，不对外网暴露。

输入法本身不主动发起任何网络请求。

## 使用条款

使用本软件即表示您知悉并同意以下约定：

- 本软件为本地工具，安装、配置与使用风险由用户自行承担。
- 请仅将本软件用于合法用途；请勿用于干扰系统安全或侵犯他人权益的场景。
- 词库与学习数据归用户本地管理；卸载前请自行备份需要保留的用户词典。
- 软件可能通过官网发布更新；是否安装更新由用户决定。

完整法律文本以安装目录 LICENSE.txt（GNU Affero General Public License v3.0 or later）为准。官网：https://www.babelspan.com/

## 开放 API

开放 API 面向「用 AI 帮助用户定制个性化输入法」的场景：本机 Agent / 脚本可通过本地接口查询状态、调整配置、访问词库相关能力，从而按个人习惯协助设置输入法。

默认关闭。需要 Agent 接入时请先开启：

```cmd
prisir_tsfsvc --open-api-config enable
```

开启后仅监听本机 127.0.0.1（默认端口 23331），不对外网开放。配置文件：`%LOCALAPPDATA%\Prisir\IME\open_api.json`。

常用端点：`GET /health` · `GET /status` · `POST /rpc`（JSON-RPC）。

关闭：`prisir_tsfsvc --open-api-config disable`  
查看：`prisir_tsfsvc --open-api-config show`  
说明：`prisir_tsfsvc --about openapi`

请仅在信任本机环境时开启；不需要个性化 Agent 协助时可保持关闭。

## 问题反馈

如需报告缺陷或提出改进建议，请通过 BabelSpan 论坛提交：

https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai

建议在帖中说明：Windows 版本与灵犀版本号、复现步骤、期望与实际结果。

状态栏菜单「问题反馈」将自动在桌面生成诊断包（日志与脱敏环境信息，不含输入正文），并打开论坛页面，便于上传附件。

---

更详细的安装与验证步骤见 INSTALL.md。
