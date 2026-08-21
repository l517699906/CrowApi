import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, KeyRound, RefreshCcw, ShieldCheck } from "lucide-react";
import { secretApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { MasterKeyRotationResult } from "../types";
import { Modal } from "./ui";

interface MasterKeySettingsPanelProps {
    onNotice: (message: string) => void;
}

export function MasterKeySettingsPanel({ onNotice }: MasterKeySettingsPanelProps) {
    const queryClient = useQueryClient();
    const [confirming, setConfirming] = useState(false);
    const [lastRotation, setLastRotation] = useState<MasterKeyRotationResult | null>(null);
    const statusQuery = useQuery({
        queryKey: queryKeys.masterKeyStatus,
        queryFn: secretApi.getMasterKeyStatus,
    });
    const rotation = useMutation({
        mutationFn: secretApi.rotateMasterKey,
        onSuccess: async (result) => {
            setLastRotation(result);
            setConfirming(false);
            await queryClient.invalidateQueries({ queryKey: queryKeys.masterKeyStatus });
            onNotice(`主密钥已轮换至 v${result.activeKeyVersion}`);
        },
    });
    const status = statusQuery.data;
    const activeUsage = status?.versions.find(
        (item) => item.keyVersion === status.activeKeyVersion,
    );

    return (
        <>
            <div className="master-key-console">
                <div className="master-key-status-row">
                    <span className="master-key-mark"><KeyRound size={18} /></span>
                    <div>
                        <strong>当前活动主密钥</strong>
                        <span>系统密钥库与本地密文使用独立版本号</span>
                    </div>
                    <code>{status ? `v${status.activeKeyVersion}` : "--"}</code>
                </div>

                <div className="master-key-metrics">
                    <div><span>受保护密文</span><strong>{status?.totalSecrets ?? "--"}</strong></div>
                    <div><span>活动版本密文</span><strong>{activeUsage?.secretCount ?? 0}</strong></div>
                    <div><span>数据库密钥版本</span><strong>{status?.versions.length ?? "--"}</strong></div>
                </div>

                {status?.versions.length ? (
                    <div className="master-key-version-list" aria-label="密文密钥版本分布">
                        {status.versions.map((item) => (
                            <span key={item.keyVersion}>
                                v{item.keyVersion}<strong>{item.secretCount}</strong>
                            </span>
                        ))}
                    </div>
                ) : null}

                <div className="master-key-action-row">
                    <div>
                        <ShieldCheck size={16} />
                        <span>轮换使用单个数据库事务；旧密钥会暂时保留以兼容并发写入。</span>
                    </div>
                    <button
                        type="button"
                        className="button-primary"
                        disabled={!status || statusQuery.isPending || rotation.isPending}
                        onClick={() => setConfirming(true)}
                    >
                        {rotation.isPending ? <span className="button-spinner is-inverse" /> : <RefreshCcw size={16} />}
                        {rotation.isPending ? "正在轮换" : "轮换主密钥"}
                    </button>
                </div>

                {lastRotation ? (
                    <div className="backup-result" role="status">
                        <Check size={15} />
                        <div>
                            <strong>轮换完成</strong>
                            <code>
                                v{lastRotation.previousKeyVersion}{" -> "}v{lastRotation.activeKeyVersion}，
                                已重加密 {lastRotation.rotatedSecrets} 条
                            </code>
                        </div>
                    </div>
                ) : null}
                {statusQuery.error || rotation.error ? (
                    <p className="form-error mt-4" role="alert">
                        {errorMessage(statusQuery.error ?? rotation.error)}
                    </p>
                ) : null}
            </div>

            {confirming ? (
                <Modal
                    title="确认轮换主密钥"
                    description="CrowAPI 将逐条验证并重加密所有受保护密文。任意一条失败都会整体回滚。"
                    size="sm"
                    onClose={() => {
                        if (!rotation.isPending) setConfirming(false);
                    }}
                    footer={(
                        <>
                            <button
                                type="button"
                                className="button-secondary"
                                disabled={rotation.isPending}
                                onClick={() => setConfirming(false)}
                            >
                                取消
                            </button>
                            <button
                                type="button"
                                className="button-primary"
                                disabled={rotation.isPending}
                                onClick={() => rotation.mutate()}
                            >
                                {rotation.isPending ? <span className="button-spinner is-inverse" /> : <RefreshCcw size={16} />}
                                {rotation.isPending ? "正在验证并轮换" : "确认轮换"}
                            </button>
                        </>
                    )}
                >
                    <div className="settings-note master-key-confirm-note">
                        <ShieldCheck size={17} />
                        <span>轮换完成后，新写入密文将使用下一版本；现有备份仍可通过备份口令恢复。</span>
                    </div>
                </Modal>
            ) : null}
        </>
    );
}
