! Test for S902: TooManyParameters

module test_too_many_parameters
contains
    ! --- Cas 1 : Aucun argument ---
    subroutine no_args()
    end subroutine no_args

    ! --- Cas 2 : Arguments sur une seule ligne ---
    function three_args(a, b, c)
    end function three_args

    ! --- Cas 3 : Arguments sur plusieurs lignes ---
    function multiline_args(
        a,
        b,
        c,
        d,
        e,
        f)
    end function multiline_args

    ! --- Cas 4 : Juste au seuil ---
    subroutine at_threshold(a, b, c, d, e)
    end subroutine at_threshold

    ! --- Cas 5 : Au-dessus du seuil ---
    subroutine above_threshold(a, b, c, d, e, f)
    end subroutine above_threshold

    ! --- Cas 6 : Subroutine avec beaucoup d'arguments multiline ---
    subroutine many_args(
        a,
        b,
        c,
        d,
        e,
        f,
        g,
        h)
    end subroutine many_args

    ! --- Cas 7 : Fonction avec un seul argument ---
    function one_arg(a)
    end function one_arg

    ! --- Cas 8 : Subroutine avec des arguments optionnels ---
    subroutine optional_args(a, b, c, d, e, f, optional)
    end subroutine optional_args

end module test_too_many_parameters

! Les cas qui dépassent 5 arguments doivent déclencher S902.
! Les autres ne doivent pas déclencher.
