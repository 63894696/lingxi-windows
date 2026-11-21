# 灵犀拼音输入法 for Windows — 安装说明

本安装器(LingxiIME-...-Setup.exe)已自动完成:解压文件到
`C:\Program Files\PrisirIME\`、注册输入法(prisir_tsfsvc --register)、
重启 explorer 让输入法生效。装完用 `Win+Space` 切到「Prisir 输入法」即可。

## 手动验证 / 排查

```cmd
cd "C:\Program Files\PrisirIME"
prisir_tsfsvc.exe --status        :: 查注册状态
prisir_tsfsvc.exe --version       :: 版本号
prisir_tsfsvc.exe --about privacy :: 隐私政策
prisir_tsfsvc.exe --about openapi :: 开放 API 说明
```

## 开放 API（可选）

供本机 AI Agent 定制个性化输入法，**默认关闭**。需要 Agent 接入设置时：

```cmd
prisir_tsfsvc.exe --open-api-config enable
```

仅监听 `127.0.0.1`（默认 23331）。不需要时保持关闭即可。详见状态栏菜单「开放 API」或 `prisir_tsfsvc --about openapi`。

## 卸载

「设置 → 应用 → 已安装的应用」找 Prisir灵犀拼音,卸载;
或跑 `C:\Program Files\PrisirIME\Uninstall.exe`。
卸载会自动 --unregister 并清理文件。

## 问题反馈

状态栏菜单 →「问题反馈」→ 自动生成诊断 zip 到桌面并打开 BabelSpan 论坛，请在论坛发帖并上传附件。
