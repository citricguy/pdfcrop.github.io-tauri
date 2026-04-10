import { open, save } from '@tauri-apps/plugin-dialog';
import { readFile, writeFile } from '@tauri-apps/plugin-fs';

export interface LoadedPdfFile {
    data: Uint8Array;
    name: string;
}

export function isDesktopApp(): boolean {
    return typeof window !== 'undefined'
        && ('__TAURI_INTERNALS__' in window || '__TAURI__' in window);
}

export async function openPdfWithPlatformDialog(): Promise<LoadedPdfFile | null> {
    if (!isDesktopApp()) {
        return null;
    }

    const selectedPath = await open({
        directory: false,
        multiple: false,
        filters: [
            {
                name: 'PDF documents',
                extensions: ['pdf']
            }
        ]
    });

    if (!selectedPath || Array.isArray(selectedPath)) {
        return null;
    }

    const data = await readFile(selectedPath);
    const name = selectedPath.split(/[/\\]/).pop() || 'document.pdf';
    return { data, name };
}

export async function savePdfWithPlatformDialog(uint8Array: Uint8Array, filename: string): Promise<boolean> {
    if (isDesktopApp()) {
        const selectedPath = await save({
            defaultPath: filename,
            filters: [
                {
                    name: 'PDF documents',
                    extensions: ['pdf']
                }
            ]
        });

        if (!selectedPath) {
            return false;
        }

        await writeFile(selectedPath, uint8Array);
        return true;
    }

    // Slice to ensure we have an ArrayBuffer-backed Uint8Array (not SharedArrayBuffer)
    const blob = new Blob([uint8Array.slice()], { type: 'application/pdf' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = filename;
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
    URL.revokeObjectURL(url);
    return true;
}
