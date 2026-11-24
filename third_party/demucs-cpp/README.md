Vendored from https://github.com/sevagh/demucs.cpp commit 4edae497a114de1ccb17ac9b4a2b8fc0144714d7 (MIT, see LICENSE):
`src/` is the library only (no CLI, no libnyquist, no weights). `eigen/` is the Eigen submodule at
dd8c71e62852b2fe429edb6682ac91fd1c578a26 (MPL2, see eigen/COPYING.*), reduced to Eigen/ and the unsupported modules the library uses
(CXX11 Tensor, FFT, MatrixFunctions). Unmodified.
