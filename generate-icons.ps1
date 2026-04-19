param(
    [string]$GeneratedDir = "",
    [string]$MsixAssetsDir = ""
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing

if (-not $GeneratedDir) {
    $GeneratedDir = Join-Path $PSScriptRoot "assets\generated"
}

if (-not $MsixAssetsDir) {
    $MsixAssetsDir = Join-Path $PSScriptRoot "installer\msix\Assets"
}

function New-RoundedRectanglePath {
    param(
        [float]$X,
        [float]$Y,
        [float]$Width,
        [float]$Height,
        [float]$Radius
    )

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    if ($Radius -le 0) {
        $path.AddRectangle((New-Object System.Drawing.RectangleF($X, $Y, $Width, $Height)))
        return $path
    }

    $diameter = $Radius * 2
    $path.AddArc($X, $Y, $diameter, $diameter, 180, 90)
    $path.AddArc($X + $Width - $diameter, $Y, $diameter, $diameter, 270, 90)
    $path.AddArc($X + $Width - $diameter, $Y + $Height - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc($X, $Y + $Height - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    return $path
}

function Fill-RoundedRectangle {
    param(
        [System.Drawing.Graphics]$Graphics,
        [System.Drawing.Brush]$Brush,
        [float]$X,
        [float]$Y,
        [float]$Width,
        [float]$Height,
        [float]$Radius
    )

    $path = New-RoundedRectanglePath -X $X -Y $Y -Width $Width -Height $Height -Radius $Radius
    try {
        $Graphics.FillPath($Brush, $path)
    } finally {
        $path.Dispose()
    }
}

function Draw-RoundedRectangle {
    param(
        [System.Drawing.Graphics]$Graphics,
        [System.Drawing.Pen]$Pen,
        [float]$X,
        [float]$Y,
        [float]$Width,
        [float]$Height,
        [float]$Radius
    )

    $path = New-RoundedRectanglePath -X $X -Y $Y -Width $Width -Height $Height -Radius $Radius
    try {
        $Graphics.DrawPath($Pen, $path)
    } finally {
        $path.Dispose()
    }
}

function New-Brush {
    param(
        [int]$R,
        [int]$G,
        [int]$B,
        [int]$A = 255
    )

    return New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb($A, $R, $G, $B))
}

function New-Pen {
    param(
        [int]$R,
        [int]$G,
        [int]$B,
        [float]$Width = 1,
        [int]$A = 255
    )

    $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb($A, $R, $G, $B), $Width)
    $pen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    return $pen
}

function Draw-WtmuxSquareIcon {
    param(
        [System.Drawing.Graphics]$Graphics,
        [float]$Left,
        [float]$Top,
        [float]$Size
    )

    $state = $Graphics.Save()
    try {
        $scale = $Size / 512.0
        $Graphics.TranslateTransform($Left, $Top)
        $Graphics.ScaleTransform($scale, $scale)

        $bgBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
            (New-Object System.Drawing.PointF(64, 48)),
            (New-Object System.Drawing.PointF(448, 464)),
            [System.Drawing.Color]::FromArgb(15, 34, 48),
            [System.Drawing.Color]::FromArgb(7, 20, 29)
        )
        $bgBlend = New-Object System.Drawing.Drawing2D.ColorBlend
        $bgBlend.Colors = [System.Drawing.Color[]]@(
            [System.Drawing.Color]::FromArgb(15, 34, 48),
            [System.Drawing.Color]::FromArgb(17, 56, 74),
            [System.Drawing.Color]::FromArgb(7, 20, 29)
        )
        $bgBlend.Positions = [float[]]@(0.0, 0.55, 1.0)
        $bgBrush.InterpolationColors = $bgBlend

        $panelBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
            (New-Object System.Drawing.PointF(84, 84)),
            (New-Object System.Drawing.PointF(428, 428)),
            [System.Drawing.Color]::FromArgb(19, 45, 60),
            [System.Drawing.Color]::FromArgb(11, 27, 38)
        )

        $accentBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
            (New-Object System.Drawing.PointF(96, 132)),
            (New-Object System.Drawing.PointF(272, 300)),
            [System.Drawing.Color]::FromArgb(82, 242, 197),
            [System.Drawing.Color]::FromArgb(65, 184, 255)
        )

        $separatorBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
            (New-Object System.Drawing.PointF(256, 132)),
            (New-Object System.Drawing.PointF(256, 404)),
            [System.Drawing.Color]::FromArgb(242, 87, 223, 255),
            [System.Drawing.Color]::FromArgb(76, 87, 223, 255)
        )

        $chromeBrush = New-Brush 23 56 74
        $windowButtonBrush = New-Brush 27 67 87
        $closeButtonBrush = New-Brush 140 47 69
        $appGlyphBrush = New-Brush 82 242 197 230
        $titleBrush = New-Brush 138 223 255 46
        $panelStrokeBrush = New-Pen 148 230 255 2 36
        $innerStrokePen = New-Pen 95 224 255 2 31
        $paneFillBrush = New-Brush 13 28 39
        $paneStrokePen = New-Pen 138 223 255 3 71
        $paneAccentPen = New-Object System.Drawing.Pen($accentBrush, 4)
        $paneAccentPen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
        $paneAccentPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
        $paneAccentPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round

        $cyanBrush = New-Brush 217 247 255
        $softCyanBrush = New-Brush 156 239 255
        $midBlueBrush = New-Brush 83 192 255 191
        $midBlueSoftBrush = New-Brush 83 192 255 115
        $cursorBrush = New-Brush 82 242 197
        $closeGlyphPen = New-Pen 255 243 245 3.5
        $whitePen = New-Pen 217 247 255 3
        $thinWhitePen = New-Pen 223 254 247 10
        $softBluePen = New-Pen 138 223 255 7 191
        $softBluePenMuted = New-Pen 138 223 255 7 115
        $greenWavePen = New-Pen 82 242 197 12 230

        Fill-RoundedRectangle $Graphics $bgBrush 32 32 448 448 112
        Draw-RoundedRectangle $Graphics $innerStrokePen 56 56 400 400 92

        Fill-RoundedRectangle $Graphics $panelBrush 74 74 364 364 46
        Draw-RoundedRectangle $Graphics $panelStrokeBrush 74 74 364 364 46

        Fill-RoundedRectangle $Graphics $chromeBrush 74 74 364 54 46
        $Graphics.FillRectangle($chromeBrush, 74, 102, 364, 26)

        Fill-RoundedRectangle $Graphics $appGlyphBrush 98 90 18 18 5
        Fill-RoundedRectangle $Graphics $titleBrush 122 95 88 8 4

        Fill-RoundedRectangle $Graphics $windowButtonBrush 338 82 28 28 8
        Fill-RoundedRectangle $Graphics $windowButtonBrush 370 82 28 28 8
        Fill-RoundedRectangle $Graphics $closeButtonBrush 402 82 28 28 8

        Fill-RoundedRectangle $Graphics $cyanBrush 346 95 12 3 1.5
        Draw-RoundedRectangle $Graphics $whitePen 378 92 12 9 1.5
        $Graphics.DrawLine($closeGlyphPen, 410, 90, 422, 102)
        $Graphics.DrawLine($closeGlyphPen, 422, 90, 410, 102)

        Fill-RoundedRectangle $Graphics $paneFillBrush 96 148 170 238 24
        Draw-RoundedRectangle $Graphics $paneAccentPen 96 148 170 238 24

        Fill-RoundedRectangle $Graphics $paneFillBrush 286 148 130 98 20
        Draw-RoundedRectangle $Graphics $paneStrokePen 286 148 130 98 20

        Fill-RoundedRectangle $Graphics $paneFillBrush 286 266 130 120 20
        Draw-RoundedRectangle $Graphics $paneStrokePen 286 266 130 120 20

        Fill-RoundedRectangle $Graphics $separatorBrush 274 148 4 238 2
        Fill-RoundedRectangle $Graphics $separatorBrush 286 254 130 4 2

        Fill-RoundedRectangle $Graphics $cursorBrush 116 176 28 10 5
        $Graphics.DrawLines($thinWhitePen, [System.Drawing.PointF[]]@(
            (New-Object System.Drawing.PointF(126, 214)),
            (New-Object System.Drawing.PointF(148, 230)),
            (New-Object System.Drawing.PointF(126, 246))
        ))

        Fill-RoundedRectangle $Graphics $softCyanBrush 162 224 58 12 6
        Fill-RoundedRectangle $Graphics $midBlueBrush 162 252 76 12 6
        Fill-RoundedRectangle $Graphics $midBlueSoftBrush 162 280 46 12 6
        Fill-RoundedRectangle $Graphics $cursorBrush 216 278 12 18 2

        $Graphics.DrawLines($softBluePen, [System.Drawing.PointF[]]@(
            (New-Object System.Drawing.PointF(308, 173)),
            (New-Object System.Drawing.PointF(327, 190)),
            (New-Object System.Drawing.PointF(308, 207))
        ))
        Fill-RoundedRectangle $Graphics (New-Brush 138 223 255 184) 340 184 42 8 4

        $Graphics.DrawLines($softBluePenMuted, [System.Drawing.PointF[]]@(
            (New-Object System.Drawing.PointF(308, 292)),
            (New-Object System.Drawing.PointF(327, 309)),
            (New-Object System.Drawing.PointF(308, 326))
        ))
        Fill-RoundedRectangle $Graphics (New-Brush 138 223 255 97) 340 304 50 8 4

        $Graphics.DrawLines($greenWavePen, [System.Drawing.PointF[]]@(
            (New-Object System.Drawing.PointF(135, 350)),
            (New-Object System.Drawing.PointF(161, 316)),
            (New-Object System.Drawing.PointF(188, 350)),
            (New-Object System.Drawing.PointF(214, 316)),
            (New-Object System.Drawing.PointF(241, 350))
        ))

        $bgBrush.Dispose()
        $panelBrush.Dispose()
        $accentBrush.Dispose()
        $separatorBrush.Dispose()
        $chromeBrush.Dispose()
        $windowButtonBrush.Dispose()
        $closeButtonBrush.Dispose()
        $appGlyphBrush.Dispose()
        $titleBrush.Dispose()
        $panelStrokeBrush.Dispose()
        $innerStrokePen.Dispose()
        $paneFillBrush.Dispose()
        $paneStrokePen.Dispose()
        $paneAccentPen.Dispose()
        $cyanBrush.Dispose()
        $softCyanBrush.Dispose()
        $midBlueBrush.Dispose()
        $midBlueSoftBrush.Dispose()
        $cursorBrush.Dispose()
        $closeGlyphPen.Dispose()
        $whitePen.Dispose()
        $thinWhitePen.Dispose()
        $softBluePen.Dispose()
        $softBluePenMuted.Dispose()
        $greenWavePen.Dispose()
    } finally {
        $Graphics.Restore($state)
    }
}

function New-IconBitmap {
    param(
        [int]$Width,
        [int]$Height,
        [switch]$Wide
    )

    $bitmap = New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit

    try {
        if ($Wide) {
            $backgroundBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
                (New-Object System.Drawing.PointF(0, 0)),
                (New-Object System.Drawing.PointF($Width, $Height)),
                [System.Drawing.Color]::FromArgb(15, 34, 48),
                [System.Drawing.Color]::FromArgb(7, 20, 29)
            )
            $backgroundBlend = New-Object System.Drawing.Drawing2D.ColorBlend
            $backgroundBlend.Colors = [System.Drawing.Color[]]@(
                [System.Drawing.Color]::FromArgb(15, 34, 48),
                [System.Drawing.Color]::FromArgb(17, 56, 74),
                [System.Drawing.Color]::FromArgb(7, 20, 29)
            )
            $backgroundBlend.Positions = [float[]]@(0.0, 0.55, 1.0)
            $backgroundBrush.InterpolationColors = $backgroundBlend

            Fill-RoundedRectangle $graphics $backgroundBrush 0 0 $Width $Height 24
            Draw-WtmuxSquareIcon -Graphics $graphics -Left 18 -Top 17 -Size 116

            $titleBrush = New-Brush 217 247 255
            $accentBrush = New-Brush 82 242 197
            $titleFont = New-Object System.Drawing.Font("Segoe UI Semibold", 30, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
            $subtitleFont = New-Object System.Drawing.Font("Segoe UI", 12, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)

            $graphics.DrawString("wtmux", $titleFont, $titleBrush, 152, 42)
            $graphics.DrawString("windows terminal multiplexer", $subtitleFont, $accentBrush, 154, 86)

            $backgroundBrush.Dispose()
            $titleBrush.Dispose()
            $accentBrush.Dispose()
            $titleFont.Dispose()
            $subtitleFont.Dispose()
        } else {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            Draw-WtmuxSquareIcon -Graphics $graphics -Left 0 -Top 0 -Size ([Math]::Min($Width, $Height))
        }

        return $bitmap
    } finally {
        $graphics.Dispose()
    }
}

function Save-BitmapPng {
    param(
        [System.Drawing.Bitmap]$Bitmap,
        [string]$Path
    )

    $directory = Split-Path -Parent $Path
    if ($directory -and -not (Test-Path $directory)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
}

function Write-IcoFile {
    param(
        [int[]]$Sizes,
        [string]$Path
    )

    $entries = @()
    foreach ($size in $Sizes) {
        $bitmap = New-IconBitmap -Width $size -Height $size
        try {
            $stream = New-Object System.IO.MemoryStream
            try {
                $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
                $entries += [PSCustomObject]@{
                    Size = $size
                    Data = $stream.ToArray()
                }
            } finally {
                $stream.Dispose()
            }
        } finally {
            $bitmap.Dispose()
        }
    }

    $directory = Split-Path -Parent $Path
    if ($directory -and -not (Test-Path $directory)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }

    $fileStream = [System.IO.File]::Create($Path)
    try {
        $writer = New-Object System.IO.BinaryWriter($fileStream)
        try {
            $writer.Write([uint16]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]$entries.Count)

            $offset = 6 + (16 * $entries.Count)
            foreach ($entry in $entries) {
                $sizeByte = if ($entry.Size -ge 256) { 0 } else { $entry.Size }
                $writer.Write([byte]$sizeByte)
                $writer.Write([byte]$sizeByte)
                $writer.Write([byte]0)
                $writer.Write([byte]0)
                $writer.Write([uint16]1)
                $writer.Write([uint16]32)
                $writer.Write([uint32]$entry.Data.Length)
                $writer.Write([uint32]$offset)
                $offset += $entry.Data.Length
            }

            foreach ($entry in $entries) {
                $writer.Write($entry.Data)
            }
        } finally {
            $writer.Dispose()
        }
    } finally {
        $fileStream.Dispose()
    }
}

Write-Host "Generating wtmux icons..." -ForegroundColor Cyan

New-Item -ItemType Directory -Path $GeneratedDir -Force | Out-Null
New-Item -ItemType Directory -Path $MsixAssetsDir -Force | Out-Null

$square150 = New-IconBitmap -Width 150 -Height 150
try {
    Save-BitmapPng -Bitmap $square150 -Path (Join-Path $MsixAssetsDir "Square150x150Logo.png")
} finally {
    $square150.Dispose()
}

$square44 = New-IconBitmap -Width 44 -Height 44
try {
    Save-BitmapPng -Bitmap $square44 -Path (Join-Path $MsixAssetsDir "Square44x44Logo.png")
} finally {
    $square44.Dispose()
}

$store50 = New-IconBitmap -Width 50 -Height 50
try {
    Save-BitmapPng -Bitmap $store50 -Path (Join-Path $MsixAssetsDir "StoreLogo.png")
} finally {
    $store50.Dispose()
}

$wideTile = New-IconBitmap -Width 310 -Height 150 -Wide
try {
    Save-BitmapPng -Bitmap $wideTile -Path (Join-Path $MsixAssetsDir "Wide310x150Logo.png")
} finally {
    $wideTile.Dispose()
}

$preview256 = New-IconBitmap -Width 256 -Height 256
try {
    Save-BitmapPng -Bitmap $preview256 -Path (Join-Path $GeneratedDir "wtmux-256.png")
} finally {
    $preview256.Dispose()
}

Write-IcoFile -Sizes @(16, 24, 32, 48, 64, 128, 256) -Path (Join-Path $GeneratedDir "wtmux.ico")

Write-Host "Generated icons:" -ForegroundColor Green
Write-Host "  $(Join-Path $GeneratedDir 'wtmux.ico')" -ForegroundColor Gray
Write-Host "  $(Join-Path $GeneratedDir 'wtmux-256.png')" -ForegroundColor Gray
Write-Host "  $MsixAssetsDir" -ForegroundColor Gray
