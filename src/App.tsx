import { useEffect, useMemo, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const GROUP_WINDOW_MS = 2 * 60 * 1000;
const MAX_GROUP_IMAGES = 5;

type DetectedImage = {
  path: string;
  detectedAt: number;
};

type ImageGroup = {
  id: string;
  createdAt: number;
  images: DetectedImage[];
};

type ImageStat = {
  path: string;
  averageLuma: number;
};

type MergeResult = {
  outputPngPath: string;
  outputExrPath?: string | null;
  width: number;
  height: number;
  mergedAt: string;
};

export default function App() {
  const [watchFolder, setWatchFolder] = useState("");
  const [isWatching, setIsWatching] = useState(false);
  const [status, setStatus] = useState("停止中");
  const [groups, setGroups] = useState<ImageGroup[]>([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [stats, setStats] = useState<ImageStat[]>([]);
  const [mergeResult, setMergeResult] = useState<MergeResult | null>(null);
  const [includeExr, setIncludeExr] = useState(true);
  const [isMerging, setIsMerging] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const selectedGroup = useMemo(
    () => groups.find((group) => group.id === selectedGroupId) ?? null,
    [groups, selectedGroupId]
  );

  const exposureDelta = useMemo(() => {
    if (stats.length === 0) {
      return null;
    }
    const values = stats.map((stat) => stat.averageLuma);
    const min = Math.min(...values);
    const max = Math.max(...values);
    return max - min;
  }, [stats]);

  useEffect(() => {
    const setup = async () => {
      const running = await invoke<boolean>("watcher_is_running");
      setIsWatching(running);
      setStatus(running ? "監視中" : "停止中");
    };

    setup();
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<string>("hdr://file-detected", (event) => {
      const path = event.payload;
      const detectedAt = Date.now();

      setGroups((prev) => {
        const alreadyExists = prev.some((group) =>
          group.images.some((image) => image.path === path)
        );
        if (alreadyExists) {
          return prev;
        }

        const nextGroups = [...prev];
        const lastGroup = nextGroups[nextGroups.length - 1];

        if (
          lastGroup &&
          lastGroup.images.length < MAX_GROUP_IMAGES &&
          detectedAt - lastGroup.images[lastGroup.images.length - 1].detectedAt <=
            GROUP_WINDOW_MS
        ) {
          lastGroup.images = [...lastGroup.images, { path, detectedAt }];
          return nextGroups;
        }

        const newGroup: ImageGroup = {
          id: crypto.randomUUID(),
          createdAt: detectedAt,
          images: [{ path, detectedAt }]
        };
        return [...nextGroups, newGroup];
      });
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const handleStart = async () => {
    if (!watchFolder.trim()) {
      setMessage("監視フォルダを指定してください");
      return;
    }

    try {
      await invoke("watcher_set_folder", { folder: watchFolder.trim() });
      await invoke("watcher_start");
      setIsWatching(true);
      setStatus("監視中");
      setMessage(null);
    } catch (error) {
      setMessage(String(error));
    }
  };

  const handleStop = async () => {
    try {
      await invoke("watcher_stop");
      setIsWatching(false);
      setStatus("停止中");
    } catch (error) {
      setMessage(String(error));
    }
  };

  const handleAnalyze = async () => {
    if (!selectedGroup) {
      setMessage("連続撮影グループを選択してください");
      return;
    }

    try {
      const result = await invoke<ImageStat[]>("analyze_images", {
        paths: selectedGroup.images.map((image) => image.path)
      });
      setStats(result);
      setMessage(null);
    } catch (error) {
      setMessage(String(error));
    }
  };

  const handleMerge = async () => {
    if (!selectedGroup) {
      setMessage("連続撮影グループを選択してください");
      return;
    }
    if (selectedGroup.images.length < 2) {
      setMessage("合成には最低2枚必要です");
      return;
    }

    setIsMerging(true);
    setMessage(null);
    setMergeResult(null);

    try {
      const result = await invoke<MergeResult>("merge_hdr", {
        request: {
          paths: selectedGroup.images.map((image) => image.path),
          outputDir: watchFolder.trim() || null,
          outputExr: includeExr
        }
      });
      setMergeResult(result);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setIsMerging(false);
    }
  };

  const previewUrl = mergeResult?.outputPngPath
    ? convertFileSrc(mergeResult.outputPngPath)
    : null;

  return (
    <div className="app">
      <div className="header">
        <div>
          <div className="title">HDR 合成プロトタイプ</div>
          <div className="muted">監視 → 連続撮影検出 → 合成 → 比較</div>
        </div>
        <div className="badge">状態: {status}</div>
      </div>

      <div className="card">
        <div className="row">
          <input
            className="input"
            value={watchFolder}
            onChange={(event) => setWatchFolder(event.target.value)}
            placeholder="監視フォルダを入力"
          />
          <button className="button" onClick={handleStart} disabled={isWatching}>
            監視開始
          </button>
          <button
            className="button secondary"
            onClick={handleStop}
            disabled={!isWatching}
          >
            監視停止
          </button>
        </div>
        {message && <div className="muted">{message}</div>}
        <div className="muted">連続撮影の判定: {GROUP_WINDOW_MS / 60000}分以内</div>
      </div>

      <div className="grid">
        <div className="card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <strong>連続撮影グループ</strong>
            <span className="muted">最大 {MAX_GROUP_IMAGES} 枚まで自動追加</span>
          </div>
          <div className="list">
            {groups.length === 0 && <div className="muted">未検出</div>}
            {groups.map((group) => (
              <div
                key={group.id}
                className={`group ${group.id === selectedGroupId ? "active" : ""}`}
                onClick={() => {
                  setSelectedGroupId(group.id);
                  setStats([]);
                  setMergeResult(null);
                }}
              >
                <div>画像 {group.images.length} 枚</div>
                <div className="muted small">
                  {new Date(group.createdAt).toLocaleString()}
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="card">
          <strong>解析 / 合成</strong>
          <div className="row" style={{ marginTop: 12 }}>
            <label className="row">
              <input
                type="checkbox"
                checked={includeExr}
                onChange={(event) => setIncludeExr(event.target.checked)}
              />
              <span className="small">EXRも出力する</span>
            </label>
          </div>
          <div className="row" style={{ marginTop: 12 }}>
            <button className="button secondary" onClick={handleAnalyze}>
              露光差を解析
            </button>
            <button className="button" onClick={handleMerge} disabled={isMerging}>
              {isMerging ? "合成中..." : "合成実行"}
            </button>
          </div>
          <div className="muted" style={{ marginTop: 8 }}>
            {selectedGroup
              ? `選択中: ${selectedGroup.images.length} 枚`
              : "グループを選択してください"}
          </div>

          {stats.length > 0 && (
            <div style={{ marginTop: 12 }}>
              <div className="small">平均輝度（0〜1）</div>
              <ul className="small">
                {stats.map((stat) => (
                  <li key={stat.path}>
                    {stat.averageLuma.toFixed(4)}
                    <span className="muted"> - {stat.path}</span>
                  </li>
                ))}
              </ul>
              {exposureDelta !== null && (
                <div className="muted">露光差: {exposureDelta.toFixed(4)}</div>
              )}
            </div>
          )}
        </div>
      </div>

      <div className="card">
        <strong>合成結果</strong>
        {mergeResult ? (
          <div className="preview" style={{ marginTop: 12 }}>
            <div>
              <div className="small">16bit PNG</div>
              {previewUrl && <img src={previewUrl} alt="HDR PNG" />}
              <div className="muted small">{mergeResult.outputPngPath}</div>
            </div>
            <div>
              <div className="small">EXR</div>
              <div className="muted small">
                {mergeResult.outputExrPath ?? "EXR出力なし"}
              </div>
            </div>
          </div>
        ) : (
          <div className="muted" style={{ marginTop: 8 }}>
            合成結果はここに表示されます
          </div>
        )}
      </div>
    </div>
  );
}
