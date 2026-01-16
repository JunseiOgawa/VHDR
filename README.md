# VHDR

## セットアップ（Bun）
1. 依存関係をインストール
	- `bun install`
2. フロントエンド開発サーバー
	- `bun run dev`
3. Tauri起動
	- `bun run tauri dev`

## 概要
HDR合成の簡易PoCアプリです。監視フォルダから連続撮影画像を検出し、平均合成で16bit PNGとEXRを出力します。

## 前提
- Bun
- Rust (Tauri)

## 使い方
1. 監視フォルダを入力
2. 「監視開始」を押す
3. 連続撮影グループが自動でリストに表示される
4. グループを選択して「露光差を解析」または「合成実行」
5. 合成結果の16bit PNGとEXR（任意）が出力される

## 仕様メモ
- 連続撮影の判定: 2分以内の撮影を同グループとして扱います
- 最大5枚までを1グループに含めます
- EXRはUIの「EXRも出力する」チェックで有効化します

## 設定/実装
- 監視・合成ロジック: [src-tauri/src/lib.rs](src-tauri/src/lib.rs)
- UI: [src/App.tsx](src/App.tsx)