/**
 * Interactive BBox Selection Overlay
 *
 * Handles pointer interactions for drawing, resizing, and moving bounding
 * boxes on the PDF canvas overlay.
 */

import type { PDFViewer, PDFBBox } from './pdf-viewer';

interface CanvasRect {
    x: number;
    y: number;
    width: number;
    height: number;
}

interface CanvasPoint {
    x: number;
    y: number;
}

type InteractionMode = 'draw' | 'resize' | 'move' | null;
type ResizeHandle = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';

interface BBoxOverlayOptions {
    onBboxChange?: ((bbox: PDFBBox | null) => void) | null;
    onBboxComplete?: ((bbox: PDFBBox | null) => void) | null;
    strokeStyle?: string;
    fillStyle?: string;
    lineWidth?: number;
    lineDash?: number[];
}

export class BBoxOverlay {
    private overlayCanvas: HTMLCanvasElement;
    private pdfViewer: PDFViewer;
    private ctx: CanvasRenderingContext2D;

    private startX: number = 0;
    private startY: number = 0;
    private currentX: number = 0;
    private currentY: number = 0;
    private currentBbox: PDFBBox | null = null;

    private activePointerId: number | null = null;
    private interactionMode: InteractionMode = null;
    private activeHandle: ResizeHandle | null = null;
    private interactionStartPoint: CanvasPoint | null = null;
    private interactionStartRect: CanvasRect | null = null;

    private onBboxChange: ((bbox: PDFBBox | null) => void) | null;
    private onBboxComplete: ((bbox: PDFBBox | null) => void) | null;

    private strokeStyle: string;
    private fillStyle: string;
    private lineWidth: number;
    private lineDash: number[];

    private readonly handleSize: number = 40;
    private readonly handleHitSlop: number = 18;
    private readonly handleInset: number = 20;

    private handlePointerDown: (e: PointerEvent) => void;
    private handlePointerMove: (e: PointerEvent) => void;
    private handlePointerUp: (e: PointerEvent) => void;
    private handlePointerCancel: (e: PointerEvent) => void;

    constructor(overlayCanvas: HTMLCanvasElement, pdfViewer: PDFViewer, options: BBoxOverlayOptions = {}) {
        this.overlayCanvas = overlayCanvas;
        this.pdfViewer = pdfViewer;

        const ctx = overlayCanvas.getContext('2d');
        if (!ctx) {
            throw new Error('Failed to get overlay canvas 2D context');
        }
        this.ctx = ctx;

        this.onBboxChange = options.onBboxChange || null;
        this.onBboxComplete = options.onBboxComplete || null;
        this.strokeStyle = options.strokeStyle || '#0ea5e9';
        this.fillStyle = options.fillStyle || 'rgba(14, 165, 233, 0.15)';
        this.lineWidth = options.lineWidth || 2;
        this.lineDash = options.lineDash || [];

        this.handlePointerDown = this.onPointerDown.bind(this);
        this.handlePointerMove = this.onPointerMove.bind(this);
        this.handlePointerUp = this.onPointerUp.bind(this);
        this.handlePointerCancel = this.onPointerCancel.bind(this);

        this.enable();
    }

    enable(): void {
        this.overlayCanvas.style.pointerEvents = 'auto';
        this.overlayCanvas.style.touchAction = 'none';
        this.overlayCanvas.style.cursor = 'crosshair';
        this.overlayCanvas.addEventListener('pointerdown', this.handlePointerDown);
        this.overlayCanvas.addEventListener('pointermove', this.handlePointerMove);
        this.overlayCanvas.addEventListener('pointerup', this.handlePointerUp);
        this.overlayCanvas.addEventListener('pointercancel', this.handlePointerCancel);
    }

    disable(): void {
        if (this.activePointerId !== null && this.overlayCanvas.hasPointerCapture(this.activePointerId)) {
            this.overlayCanvas.releasePointerCapture(this.activePointerId);
        }

        this.overlayCanvas.removeEventListener('pointerdown', this.handlePointerDown);
        this.overlayCanvas.removeEventListener('pointermove', this.handlePointerMove);
        this.overlayCanvas.removeEventListener('pointerup', this.handlePointerUp);
        this.overlayCanvas.removeEventListener('pointercancel', this.handlePointerCancel);
        this.overlayCanvas.style.cursor = 'default';
        this.resetInteraction();
    }

    private getPointerCoordinates(event: PointerEvent): CanvasPoint {
        const rect = this.overlayCanvas.getBoundingClientRect();
        return {
            x: event.clientX - rect.left,
            y: event.clientY - rect.top
        };
    }

    private clampToCanvas(point: CanvasPoint): CanvasPoint {
        const rect = this.overlayCanvas.getBoundingClientRect();
        return {
            x: Math.max(0, Math.min(point.x, rect.width)),
            y: Math.max(0, Math.min(point.y, rect.height))
        };
    }

    private onPointerDown(event: PointerEvent): void {
        if (!event.isPrimary) {
            return;
        }

        if (event.pointerType === 'mouse' && event.button !== 0) {
            return;
        }

        event.preventDefault();

        const point = this.clampToCanvas(this.getPointerCoordinates(event));
        const rect = this.getSelectionRect();
        const handle = rect ? this.getHandleAtPoint(point, rect) : null;
        const isInsideRect = rect ? this.isPointInsideRect(point, rect) : false;

        this.activePointerId = event.pointerId;
        this.overlayCanvas.setPointerCapture(event.pointerId);
        this.interactionStartPoint = point;
        this.interactionStartRect = rect;

        if (handle) {
            this.interactionMode = 'resize';
            this.activeHandle = handle;
        } else if (isInsideRect) {
            this.interactionMode = 'move';
            this.activeHandle = null;
        } else {
            this.interactionMode = 'draw';
            this.activeHandle = null;
            this.currentBbox = null;
            this.startX = point.x;
            this.startY = point.y;
            this.currentX = point.x;
            this.currentY = point.y;
            this.clearCanvas();
        }
    }

    private onPointerMove(event: PointerEvent): void {
        const point = this.clampToCanvas(this.getPointerCoordinates(event));

        if (this.activePointerId !== event.pointerId || this.interactionMode === null) {
            this.updateCursor(point);
            return;
        }

        event.preventDefault();
        this.updateInteraction(point);
    }

    private onPointerUp(event: PointerEvent): void {
        if (this.activePointerId !== event.pointerId) {
            return;
        }

        event.preventDefault();
        const point = this.clampToCanvas(this.getPointerCoordinates(event));
        this.finishInteraction(point);
    }

    private onPointerCancel(event: PointerEvent): void {
        if (this.activePointerId !== event.pointerId) {
            return;
        }

        this.finishInteraction(null);
    }

    private updateInteraction(point: CanvasPoint): void {
        if (this.interactionMode === 'draw') {
            this.currentX = point.x;
            this.currentY = point.y;
        } else if (this.interactionMode === 'resize' && this.interactionStartRect && this.activeHandle) {
            this.applyCanvasRect(this.resizeRect(this.interactionStartRect, this.activeHandle, point));
        } else if (this.interactionMode === 'move' && this.interactionStartRect && this.interactionStartPoint) {
            const deltaX = point.x - this.interactionStartPoint.x;
            const deltaY = point.y - this.interactionStartPoint.y;
            this.applyCanvasRect(this.moveRect(this.interactionStartRect, deltaX, deltaY));
        } else {
            return;
        }

        this.draw();
        this.emitBboxChange();
    }

    private finishInteraction(point: CanvasPoint | null): void {
        if (this.interactionMode !== null && point) {
            this.updateInteraction(point);
        }

        if (this.interactionMode === null) {
            this.releasePointerCapture();
            this.resetInteraction();
            return;
        }

        const rect = this.getCanvasBbox();
        this.currentBbox = rect.width > 0 && rect.height > 0
            ? this.canvasBboxToPdf(rect)
            : null;

        if (this.currentBbox) {
            this.draw();
        } else {
            this.clearCanvas();
        }

        if (this.onBboxComplete) {
            this.onBboxComplete(this.currentBbox);
        }

        this.releasePointerCapture();
        this.resetInteraction();
        this.updateCursor(point ?? this.getFallbackPoint());
    }

    private releasePointerCapture(): void {
        if (this.activePointerId !== null && this.overlayCanvas.hasPointerCapture(this.activePointerId)) {
            this.overlayCanvas.releasePointerCapture(this.activePointerId);
        }
    }

    private resetInteraction(): void {
        this.activePointerId = null;
        this.interactionMode = null;
        this.activeHandle = null;
        this.interactionStartPoint = null;
        this.interactionStartRect = null;
    }

    private getFallbackPoint(): CanvasPoint {
        return {
            x: this.currentX,
            y: this.currentY
        };
    }

    private hasSelection(): boolean {
        return this.currentBbox !== null;
    }

    private getSelectionRect(): CanvasRect | null {
        if (!this.hasSelection()) {
            return null;
        }

        return this.getCanvasBbox();
    }

    private applyCanvasRect(rect: CanvasRect): void {
        this.startX = rect.x;
        this.startY = rect.y;
        this.currentX = rect.x + rect.width;
        this.currentY = rect.y + rect.height;
    }

    private normalizeRect(x1: number, y1: number, x2: number, y2: number): CanvasRect {
        return {
            x: Math.min(x1, x2),
            y: Math.min(y1, y2),
            width: Math.abs(x2 - x1),
            height: Math.abs(y2 - y1)
        };
    }

    private getHandlePositions(rect: CanvasRect): Record<ResizeHandle, CanvasPoint> {
        const insetX = Math.min(this.handleInset, rect.width / 2);
        const insetY = Math.min(this.handleInset, rect.height / 2);

        return {
            'top-left': { x: rect.x + insetX, y: rect.y + insetY },
            'top-right': { x: rect.x + rect.width - insetX, y: rect.y + insetY },
            'bottom-left': { x: rect.x + insetX, y: rect.y + rect.height - insetY },
            'bottom-right': { x: rect.x + rect.width - insetX, y: rect.y + rect.height - insetY }
        };
    }

    private getHandleAtPoint(point: CanvasPoint, rect: CanvasRect): ResizeHandle | null {
        const hitRadius = (this.handleSize / 2) + this.handleHitSlop;
        const handles = this.getHandlePositions(rect);

        for (const [handle, handlePoint] of Object.entries(handles) as Array<[ResizeHandle, CanvasPoint]>) {
            if (
                Math.abs(point.x - handlePoint.x) <= hitRadius &&
                Math.abs(point.y - handlePoint.y) <= hitRadius
            ) {
                return handle;
            }
        }

        return null;
    }

    private isPointInsideRect(point: CanvasPoint, rect: CanvasRect): boolean {
        return (
            point.x >= rect.x &&
            point.x <= rect.x + rect.width &&
            point.y >= rect.y &&
            point.y <= rect.y + rect.height
        );
    }

    private resizeRect(rect: CanvasRect, handle: ResizeHandle, point: CanvasPoint): CanvasRect {
        const left = rect.x;
        const right = rect.x + rect.width;
        const top = rect.y;
        const bottom = rect.y + rect.height;

        switch (handle) {
            case 'top-left':
                return this.normalizeRect(point.x, point.y, right, bottom);
            case 'top-right':
                return this.normalizeRect(left, point.y, point.x, bottom);
            case 'bottom-left':
                return this.normalizeRect(point.x, top, right, point.y);
            case 'bottom-right':
                return this.normalizeRect(left, top, point.x, point.y);
        }
    }

    private moveRect(rect: CanvasRect, deltaX: number, deltaY: number): CanvasRect {
        const bounds = this.overlayCanvas.getBoundingClientRect();
        const maxX = Math.max(0, bounds.width - rect.width);
        const maxY = Math.max(0, bounds.height - rect.height);
        const nextX = Math.max(0, Math.min(rect.x + deltaX, maxX));
        const nextY = Math.max(0, Math.min(rect.y + deltaY, maxY));

        return {
            x: nextX,
            y: nextY,
            width: rect.width,
            height: rect.height
        };
    }

    private emitBboxChange(): void {
        if (!this.onBboxChange) {
            return;
        }

        this.onBboxChange(this.canvasBboxToPdf(this.getCanvasBbox()));
    }

    private updateCursor(point: CanvasPoint): void {
        const rect = this.getSelectionRect();
        if (!rect) {
            this.overlayCanvas.style.cursor = 'crosshair';
            return;
        }

        const handle = this.getHandleAtPoint(point, rect);
        if (handle === 'top-left' || handle === 'bottom-right') {
            this.overlayCanvas.style.cursor = 'nwse-resize';
            return;
        }

        if (handle === 'top-right' || handle === 'bottom-left') {
            this.overlayCanvas.style.cursor = 'nesw-resize';
            return;
        }

        if (this.isPointInsideRect(point, rect)) {
            this.overlayCanvas.style.cursor = 'move';
            return;
        }

        this.overlayCanvas.style.cursor = 'crosshair';
    }

    private getCanvasBbox(): CanvasRect {
        const rect = this.overlayCanvas.getBoundingClientRect();
        const canvasWidth = rect.width;
        const canvasHeight = rect.height;

        let x1 = Math.min(this.startX, this.currentX);
        let y1 = Math.min(this.startY, this.currentY);
        let x2 = Math.max(this.startX, this.currentX);
        let y2 = Math.max(this.startY, this.currentY);

        x1 = Math.max(0, Math.min(x1, canvasWidth));
        y1 = Math.max(0, Math.min(y1, canvasHeight));
        x2 = Math.max(0, Math.min(x2, canvasWidth));
        y2 = Math.max(0, Math.min(y2, canvasHeight));

        return {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1
        };
    }

    private canvasBboxToPdf(canvasBbox: CanvasRect): PDFBBox | null {
        return this.pdfViewer.canvasRectToPDFBbox(canvasBbox);
    }

    private clearCanvas(): void {
        this.ctx.clearRect(0, 0, this.overlayCanvas.width, this.overlayCanvas.height);
    }

    private draw(): void {
        this.clearCanvas();

        const bbox = this.getCanvasBbox();
        if (bbox.width <= 0 || bbox.height <= 0) {
            return;
        }

        this.ctx.save();
        this.ctx.strokeStyle = this.strokeStyle;
        this.ctx.fillStyle = this.fillStyle;
        this.ctx.lineWidth = this.lineWidth;
        this.ctx.setLineDash(this.lineDash);
        this.ctx.fillRect(bbox.x, bbox.y, bbox.width, bbox.height);
        this.ctx.strokeRect(bbox.x, bbox.y, bbox.width, bbox.height);
        this.ctx.restore();

        this.drawHandles(bbox);
    }

    private drawHandles(bbox: CanvasRect): void {
        const handles = Object.values(this.getHandlePositions(bbox));

        this.ctx.save();
        this.ctx.fillStyle = 'rgba(255, 255, 255, 0.12)';
        this.ctx.strokeStyle = this.strokeStyle;
        this.ctx.lineWidth = 1.5;

        for (const handle of handles) {
            this.ctx.fillRect(
                handle.x - this.handleSize / 2,
                handle.y - this.handleSize / 2,
                this.handleSize,
                this.handleSize
            );
            this.ctx.strokeRect(
                handle.x - this.handleSize / 2,
                handle.y - this.handleSize / 2,
                this.handleSize,
                this.handleSize
            );
        }

        this.ctx.restore();
    }

    clear(): void {
        this.currentBbox = null;
        this.startX = 0;
        this.startY = 0;
        this.currentX = 0;
        this.currentY = 0;
        this.resetInteraction();
        this.clearCanvas();
        this.overlayCanvas.style.cursor = 'crosshair';
    }

    setBbox(pdfBbox: PDFBBox | null): void {
        if (!pdfBbox) {
            this.clear();
            return;
        }

        this.currentBbox = pdfBbox;

        const canvasRect = this.pdfViewer.pdfBboxToCanvasRect(pdfBbox);
        this.startX = canvasRect.x;
        this.startY = canvasRect.y;
        this.currentX = canvasRect.x + canvasRect.width;
        this.currentY = canvasRect.y + canvasRect.height;

        this.draw();
    }

    getBbox(): PDFBBox | null {
        return this.currentBbox;
    }
}

export function createBBoxOverlay(overlayCanvasId: string, pdfViewer: PDFViewer, options: BBoxOverlayOptions = {}): BBoxOverlay {
    const overlayCanvas = document.getElementById(overlayCanvasId) as HTMLCanvasElement;
    if (!overlayCanvas) {
        throw new Error(`Canvas element with id "${overlayCanvasId}" not found`);
    }

    return new BBoxOverlay(overlayCanvas, pdfViewer, options);
}
