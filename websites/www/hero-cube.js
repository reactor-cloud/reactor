// Interactive three.js hero: an isometric cube of 5x5x5 cubes.
import * as THREE from 'three';

const container = document.getElementById('hero-cube');
if (container) {
  const LOOP = 60.0, G = 5, CELL = 1.15, BOX = 0.92;
  const HALF = (G - 1) * CELL * 0.5;
  const RADIUS = Math.sqrt(3) * HALF;
  const CH = 6, SLOT = 2.4;

  const COL_BASE_LO = new THREE.Color('#171717');
  const COL_BASE_HI = new THREE.Color('#3a3a3a');
  const COL_BLUE    = new THREE.Color('#d4d4d4');
  const COL_BLUE_HI = new THREE.Color('#f5f5f5');

  const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.domElement.style.width = '100%';
  renderer.domElement.style.height = '100%';
  renderer.domElement.style.display = 'block';
  container.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  let camera;
  function size() {
    const w = container.clientWidth || 1, h = container.clientHeight || 1;
    renderer.setSize(w, h, false);
    const aspect = w / h, d = 12;
    if (!camera) camera = new THREE.OrthographicCamera(-d * aspect, d * aspect, d, -d, 0.1, 200);
    else { camera.left = -d * aspect; camera.right = d * aspect; camera.top = d; camera.bottom = -d; }
    camera.position.set(16, 13, 16);
    camera.lookAt(0, 0, 0);
    const offx = aspect >= 1 ? -w * 0.3 : 0;
    camera.setViewOffset(w, h, offx, 0, w, h);
    camera.updateProjectionMatrix();
  }
  size();
  new ResizeObserver(size).observe(container);

  scene.add(new THREE.AmbientLight(0x2a2a2a, 0.45));
  const key = new THREE.DirectionalLight(0xffffff, 0.42); key.position.set(8, 14, 6); scene.add(key);
  const rim = new THREE.DirectionalLight(0x888888, 0.22); rim.position.set(-10, -4, -8); scene.add(rim);

  const group = new THREE.Group(); scene.add(group);

  const N = G * G * G;
  const geo = new THREE.BoxGeometry(BOX, BOX, BOX);
  const mat = new THREE.MeshStandardMaterial({ roughness: 0.72, metalness: 0.04 });
  mat.onBeforeCompile = (shader) => {
    shader.vertexShader =
      'varying vec3 vLocalPos;\n' +
      shader.vertexShader.replace(
        '#include <begin_vertex>',
        '#include <begin_vertex>\n  vLocalPos = position;'
      );
    shader.fragmentShader =
      'varying vec3 vLocalPos;\n' +
      shader.fragmentShader
        .replace(
          '#include <emissivemap_fragment>',
          `#include <emissivemap_fragment>
           float lit = dot(vColor, vec3(0.3, 0.59, 0.11));
           float glow = smoothstep(0.28, 0.62, lit);
           totalEmissiveRadiance += vColor * glow * 0.7;`
        )
        .replace(
          '#include <dithering_fragment>',
          `#include <dithering_fragment>
           vec3 dEdge = vec3(0.46) - abs(vLocalPos);
           float mn = min(dEdge.x, min(dEdge.y, dEdge.z));
           float mx = max(dEdge.x, max(dEdge.y, dEdge.z));
           float mid = dEdge.x + dEdge.y + dEdge.z - mn - mx;
           float edge = 1.0 - smoothstep(0.0, 0.035, mid);
           float luma = dot(gl_FragColor.rgb, vec3(0.3, 0.59, 0.11));
           vec3 edgeCol = mix(vec3(0.38, 0.44, 0.54), vec3(0.02, 0.04, 0.07), smoothstep(0.12, 0.4, luma));
           gl_FragColor.rgb = mix(gl_FragColor.rgb, edgeCol, edge * 0.75);`
        );
  };
  const mesh = new THREE.InstancedMesh(geo, mat, N);
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(N * 3), 3);
  group.add(mesh);

  const home = new Float32Array(N * 3), scatter = new Float32Array(N * 3);
  const delay = new Float32Array(N), spin = new Float32Array(N * 3), baseCol = [];
  const flyOff = new Float32Array(N * 3), flyVel = new Float32Array(N * 3), hoverLit = new Float32Array(N);

  let i = 0;
  for (let x = 0; x < G; x++) for (let y = 0; y < G; y++) for (let z = 0; z < G; z++) {
    const hx = (x - (G - 1) / 2) * CELL, hy = (y - (G - 1) / 2) * CELL, hz = (z - (G - 1) / 2) * CELL;
    home[i * 3] = hx; home[i * 3 + 1] = hy; home[i * 3 + 2] = hz;
    const dir = new THREE.Vector3().randomDirection(), r = 9 + Math.random() * 7;
    scatter[i * 3] = dir.x * r; scatter[i * 3 + 1] = dir.y * r * 0.8; scatter[i * 3 + 2] = dir.z * r;
    const rad = Math.sqrt(hx * hx + hy * hy + hz * hz) / RADIUS;
    delay[i] = rad * 2.2 + Math.random() * 0.5;
    const t = (hy + HALF) / (2 * HALF);
    baseCol[i] = COL_BASE_LO.clone().lerp(COL_BASE_HI, t * t);
    spin[i * 3] = Math.random() * 6.28; spin[i * 3 + 1] = Math.random() * 6.28; spin[i * 3 + 2] = Math.random() * 6.28;
    i++;
  }

  const perm = [], chOff = new Float32Array(CH);
  for (let c = 0; c < CH; c++) {
    const arr = Array.from({ length: N }, (_, k) => k);
    for (let j = N - 1; j > 0; j--) { const r = (Math.random() * (j + 1)) | 0; [arr[j], arr[r]] = [arr[r], arr[j]]; }
    perm.push(arr); chOff[c] = (c / CH) * SLOT + Math.random() * SLOT;
  }

  const ray = new THREE.Raycaster(), ndc = new THREE.Vector2();
  let hoverId = -1; const BRUSH = CELL * 1.6;
  function pick(ev) {
    const rect = renderer.domElement.getBoundingClientRect();
    ndc.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
    ndc.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
    ray.setFromCamera(ndc, camera);
    const hit = ray.intersectObject(mesh, false)[0];
    return hit ? hit.instanceId : -1;
  }
  renderer.domElement.addEventListener('pointermove', e => { hoverId = pick(e); });
  renderer.domElement.addEventListener('pointerleave', () => { hoverId = -1; });
  renderer.domElement.addEventListener('pointerdown', e => {
    const id = pick(e); if (id < 0) return;
    const hx = home[id * 3], hy = home[id * 3 + 1], hz = home[id * 3 + 2];
    for (let n = 0; n < N; n++) {
      const dx = home[n * 3] - hx, dy = home[n * 3 + 1] - hy, dz = home[n * 3 + 2] - hz;
      if (dx * dx + dy * dy + dz * dz <= BRUSH * BRUSH) {
        const out = new THREE.Vector3(home[n * 3], home[n * 3 + 1], home[n * 3 + 2]).normalize().multiplyScalar(8 + Math.random() * 8);
        flyVel[n * 3] = out.x + (Math.random() - 0.5) * 6;
        flyVel[n * 3 + 1] = out.y + (Math.random() - 0.5) * 6 + 3;
        flyVel[n * 3 + 2] = out.z + (Math.random() - 0.5) * 6;
      }
    }
  });

  const dummy = new THREE.Object3D(), tmpC = new THREE.Color(), lit = new Float32Array(N);
  const smooth = t => { t = Math.min(1, Math.max(0, t)); return t * t * (3 - 2 * t); };
  const clock = new THREE.Clock(); let prevTp = 0;

  function frame() {
    const dt = Math.min(0.05, clock.getDelta()), time = clock.getElapsedTime();
    const tp = time % LOOP;
    if (tp < prevTp) { flyOff.fill(0); flyVel.fill(0); }
    prevTp = tp;
    const fall = 1 - smooth((tp - (LOOP - 3)) / 3);
    group.rotation.y = time * 0.12;

    lit.fill(0);
    for (let c = 0; c < CH; c++) {
      const local = time + chOff[c], slot = Math.floor(local / SLOT), frac = local / SLOT - slot;
      const idx = perm[c][((slot % N) + N) % N];
      const env = smooth(frac / 0.18) * (1 - smooth((frac - 0.82) / 0.18));
      if (env > lit[idx]) lit[idx] = env;
    }
    for (let n = 0; n < N; n++) hoverLit[n] *= Math.exp(-dt * 5);
    if (hoverId >= 0) {
      const hx = home[hoverId * 3], hy = home[hoverId * 3 + 1], hz = home[hoverId * 3 + 2];
      for (let n = 0; n < N; n++) { const dx = home[n * 3] - hx, dy = home[n * 3 + 1] - hy, dz = home[n * 3 + 2] - hz; if (dx * dx + dy * dy + dz * dz <= BRUSH * BRUSH) hoverLit[n] = 1; }
    }

    for (let n = 0; n < N; n++) {
      const a = smooth((tp - delay[n]) / 0.8) * fall, e = smooth(a);
      flyVel[n * 3] *= (1 - dt * 1.2); flyVel[n * 3 + 1] *= (1 - dt * 1.2); flyVel[n * 3 + 2] *= (1 - dt * 1.2);
      flyOff[n * 3] += flyVel[n * 3] * dt; flyOff[n * 3 + 1] += flyVel[n * 3 + 1] * dt; flyOff[n * 3 + 2] += flyVel[n * 3 + 2] * dt;
      const fdist = Math.hypot(flyOff[n * 3], flyOff[n * 3 + 1], flyOff[n * 3 + 2]);
      dummy.position.set(
        scatter[n * 3] + (home[n * 3] - scatter[n * 3]) * e + flyOff[n * 3],
        scatter[n * 3 + 1] + (home[n * 3 + 1] - scatter[n * 3 + 1]) * e + flyOff[n * 3 + 1],
        scatter[n * 3 + 2] + (home[n * 3 + 2] - scatter[n * 3 + 2]) * e + flyOff[n * 3 + 2]);
      const tumble = (1 - a) + Math.min(1, fdist * 0.15);
      dummy.rotation.set(spin[n * 3] * tumble, spin[n * 3 + 1] * tumble, spin[n * 3 + 2] * tumble);
      const fade = Math.max(0, 1 - fdist / 22);
      dummy.scale.setScalar(Math.max(0.0001, a * fade));
      dummy.updateMatrix(); mesh.setMatrixAt(n, dummy.matrix);
      const on = Math.min(1, Math.max(lit[n], hoverLit[n])) * a;
      tmpC.copy(baseCol[n]).lerp(COL_BLUE, on);
      if (on > 0.7) tmpC.lerp(COL_BLUE_HI, (on - 0.7) / 0.3 * 0.2);
      mesh.setColorAt(n, tmpC);
    }
    mesh.instanceMatrix.needsUpdate = true;
    mesh.instanceColor.needsUpdate = true;
    renderer.render(scene, camera);
    requestAnimationFrame(frame);
  }
  frame();
}
