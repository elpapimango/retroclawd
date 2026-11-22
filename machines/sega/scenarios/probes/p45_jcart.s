; P45: a J-Cart (the header says Micro Machines 2, T-120096): the two extra pads at $380000-$3FFFFF read with TH high and low, as
; words and as bytes, and the EEPROM's data line (idle high on D7).
; mkrom: --serial T-120096
; Result at RAM offset 0: words at TH=1, TH=0, then bytes (even, odd) at TH=1, then at TH=0.
    .org $200
    lea     $FF0000,a2
    move.w  #1,$380000
    move.w  $380000,(a2)+
    move.w  #0,$380000
    move.w  $380000,(a2)+
    move.w  #1,$380000
    move.b  $380000,(a2)+
    move.b  $380001,(a2)+
    move.w  #0,$380000
    move.b  $380000,(a2)+
    move.b  $380001,(a2)+
forever:
    bra.s   forever
