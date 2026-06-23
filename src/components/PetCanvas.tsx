import { useEffect, useRef } from "react";
import type { PetManifest } from "../lib/types";
import { assetUrl } from "../lib/invoke";

interface Props {
  pet: PetManifest;
  animation: string;
  scale: number;
}

/**
 * Renders the pet's sprite sheet on a canvas, advancing frames per the active
 * animation's fps/frames. Non-looping animations (e.g. "success") hold on the
 * last frame when done.
 */
export default function PetCanvas({ pet, animation, scale }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const animRef = useRef(animation);
  const rafRef = useRef<number>(0);

  // Keep the latest animation name available to the rAF loop without
  // restarting it on every prop change.
  animRef.current = animation;

  useEffect(() => {
    const img = new Image();
    const path = pet.spritesheetFile.replace(/\\/g, "/");
    const url = assetUrl(path);
    img.src = url;
    img.onload = () => {
      imgRef.current = img;
    };
    img.onerror = () => console.warn("[claude-pet] sprite load failed", path, "→", url);
  }, [pet.spritesheetFile]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const fw = pet.frameWidth;
    const fh = pet.frameHeight;
    canvas.width = Math.round(fw * scale);
    canvas.height = Math.round(fh * scale);
    ctx.imageSmoothingEnabled = false;

    let frameIdx = 0;
    let lastTime = performance.now();
    let elapsed = 0;
    let stopped = false;

    const draw = () => {
      if (stopped) return;
      const now = performance.now();
      const dt = now - lastTime;
      lastTime = now;

      const animName = animRef.current;
      const anim = pet.animations[animName] ?? pet.animations["idle"];
      const fps = anim?.fps ?? 6;
      const frames = anim?.frames ?? [0];
      const looped = anim?.looped ?? true;
      const frameMs = 1000 / fps;

      elapsed += dt;
      while (elapsed >= frameMs) {
        elapsed -= frameMs;
        frameIdx += 1;
        if (frameIdx >= frames.length) {
          frameIdx = looped ? 0 : frames.length - 1;
        }
      }

      const img = imgRef.current;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      if (img && img.complete) {
        const frame = frames[frameIdx] ?? 0;
        const col = frame % pet.columns;
        const row = Math.floor(frame / pet.columns);
        const sx = col * fw;
        const sy = row * fh;
        ctx.drawImage(img, sx, sy, fw, fh, 0, 0, canvas.width, canvas.height);
      }
      rafRef.current = requestAnimationFrame(draw);
    };
    rafRef.current = requestAnimationFrame(draw);

    return () => {
      stopped = true;
      cancelAnimationFrame(rafRef.current);
    };
  }, [pet, scale]);

  return <canvas ref={canvasRef} className="pet-canvas" />;
}
