uniform sampler2D uTexture;
uniform vec3 uFogColor;
uniform vec3 uFogNearColor;
uniform float uFogNear;
uniform float uFogFar;

varying vec2 vUv; // u, v 
varying float vAO;
varying float vSunlight;
varying float vTorchLight;

void main() {
  vec4 textureColor = texture2D(uTexture, vUv);

  gl_FragColor = vec4(textureColor.rgb, textureColor.w);
  gl_FragColor.rgb *= (vTorchLight / 16.0 + vSunlight / 16.0 * 0.2) * vAO;

  // fog
  float depth = gl_FragCoord.z / gl_FragCoord.w;
  float fogFactor = smoothstep(uFogNear, uFogFar, depth);
  gl_FragColor.rgb = mix(gl_FragColor.rgb, mix(uFogNearColor, uFogColor, fogFactor), fogFactor);
} 