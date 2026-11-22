; P43: where in the frame the 68000 starts. The HV counter is read as the first thing the program does, twice, and the status.
; Result at RAM offset 0: HV word before and after switching to mode 5, twice more, status.
    .org $200
    lea     $FF0000,a2
    move.w  $C00008,(a2)+
    move.w  #$8104,$C00004
    move.w  $C00008,(a2)+
    move.w  $C00008,(a2)+
    move.w  $C00008,(a2)+
    move.w  $C00004,(a2)+
forever:
    bra.s   forever
