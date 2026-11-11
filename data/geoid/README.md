# Geoid grid

`egm96-15.pgm`: the EGM96 geoid on a 15-minute grid (1440 × 721 points,
90° N to 90° S, 0° E eastwards), in GeographicLib's PGM format (16-bit
big-endian samples, height = Offset + Scale × sample; Offset −108 m, Scale
0.003 m). GeographicLib converted it from the National
Geospatial-Intelligence Agency's EGM96 model (WGS-84), a US Government
work; it is distributed with GeographicLib's geoid data
(https://geographiclib.sourceforge.io/C++/doc/geoid.html).

The file's own header gives its accuracy with bilinear interpolation, which
`bgws-interop` uses: 0.040 m RMS, 1.152 m at worst.

Used only at the interoperability boundary (`crates/bgws-interop`): the
sim's heights are above mean sea level; DIS and HLA positions are above the
WGS-84 ellipsoid, and height above the ellipsoid = height above mean sea
level + the geoid undulation N.
