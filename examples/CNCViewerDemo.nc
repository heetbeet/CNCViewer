%
O1000 (MACRO COMPASS)
(Viewer sample with nominal coordinates)
(Try #100 = 6 to change the number of spokes and drill points)

G21 G17 G90 G94 G40 G49 G80 G54
#100 = 4    (Spokes)
#101 = 22   (Bolt circle radius)
#102 = 1    (Marking depth)
#103 = 650  (Contour feed)
#104 = 180  (Plunge feed)
#105 = 8    (Clearance height)
IF [#100 GE 3] GOTO 20
#3000 = 1 (Use at least three spokes)

N20 (Rounded outline)
G68 X0 Y0 R45 (Rotate the work coordinates about the hub)
G0 Z#105
G0 X-20 Y-32
G1 Z[-#102] F#104
G1 X20 F#103
G3 X32 Y-20 R12
G1 Y20
G3 X20 Y32 R12
G1 X-20
G3 X-32 Y20 R12
G1 Y-20
G3 X-20 Y-32 R12
G0 Z#105

(Two semicircles form the compass hub)
G0 X8 Y0
G1 Z[-#102] F#104
G3 X-8 Y0 R8 F#103
G3 X8 Y0 R8
G0 Z#105

(Run the spoke macro with a new angle on each visit)
#107 = 0
WHILE [#107 LT #100] DO1
  #108 = [45 + 360 * #107 / #100]
  G65 P9001 A#108 B#101 D#102 F#103 H#104 W#105
  #107 = [#107 + 1]
END1

(Drill the hub and return above the origin)
G0 X0 Y0
G98 G81 X0 Y0 Z[-3 * #102] R2 F#104
G80
G04 P250
G0 Z#105
G69
M30

O9001 (ENGRAVE ONE SPOKE AND DRILL ITS OUTER POINT)
#20 = COS[#1]
#21 = SIN[#1]
G0 Z#23
G0 X[10 * #20] Y[10 * #21]
G1 Z[-#7] F#11
G1 X[[#2 - 5] * #20] Y[[#2 - 5] * #21] F#9
G0 Z#23
G0 X[#2 * #20] Y[#2 * #21]
G98 G81 X[#2 * #20] Y[#2 * #21] Z[-2 * #7] R2 F#11
G80
M99
%
