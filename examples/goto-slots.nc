%
O1000
(GOTO demo: five parallel slots)
(Viewer example with nominal coordinates)
G21 G90 G17 G94
G54 G40 G49 G80
#100=0 (Current pass, starting at zero)
#101=5 (Number of passes)
G0 X0 Y0 Z5
N100 (Start of one pass)
G0 X0 Y[#100*10] Z5
G1 Z-1 F200
G1 X40 F600
G0 Z5
#100=#100+1
IF [#100 LT #101] GOTO 100
GOTO 900 (Jump past the unused branch)
N500 (This branch is never visited)
G1 X60 Y60 F200
N900 (All five passes finished)
G0 X0 Y0 Z5
M30
%
